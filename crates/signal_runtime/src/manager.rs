//! Live pipeline supervisor (see `docs/architecture/processing_workflows.md`).
//!
//! Unlike [`Pipeline::build`](super::pipeline::Pipeline::build), which moves
//! every channel endpoint into node threads and forgets them, the
//! `PipelineManager` *owns* each node's output subscriber lists
//! ([`SharedSenders`](super::sender::SharedSenders) behind
//! [`ErasedSharedSenders`]). That inversion is what makes partial change
//! possible while data flows:
//!
//! - **Add a tap**: subscribe new receivers into existing lists; sticky
//!   level lists prime the joiner with the current value.
//! - **Remove a branch**: unsubscribe its roots and close its own lists —
//!   the ordinary shutdown cascade, confined to the branch.
//! - **Reconfigure**: a control message applied between `work()` calls.
//! - **Restart in place**: kill via input unsubscription (the node sees a
//!   normal end-of-stream), then spawn a fresh instance wired to the *same*
//!   output lists — downstream consumers just see a quiet channel.
//!
//! A node exiting *naturally* (source finished, upstream EOS) closes its own
//! output lists on the way out, so end-of-run propagates exactly like the
//! offline drop-cascade — supervisor-driven, same semantics.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tracing::{debug, error, info};

use platform_runtime::{WorkExecutor, WorkTask};

use super::errors::WorkError;
use super::node::{
    ConfigOutcome, ConfigurationBoundary, ConfigurationScheduler, InputProtocolCandidate,
    NodeConfig, ProcessNode,
};
use super::payload_negotiation;
use super::ports::{InputPort, OutputPort, PortPayload, PortSchema, StreamSemantics};
use super::protocol::{ProtocolCapability, ProtocolKind};
use super::sender::OverflowPolicy;
use super::type_registry::{ErasedSharedSenders, TYPE_REGISTRY};
use super::watchdog::Watchdog;

/// One input wire of a node being added: which producer list to join.
#[derive(Debug, Clone)]
pub struct InputSub {
    /// Name of the producer node.
    pub from_node: String,
    /// Name of the producer output port.
    pub from_port: String,
    /// Maximum number of queued messages for this subscription.
    pub buffer: usize,
    /// Backpressure policy applied to this consumer.
    pub policy: OverflowPolicy,
}

/// Complete wiring specification for a node owned by [`PipelineManager`].
pub struct NodeSpec {
    /// Unique live-manager name of this node.
    pub name: String,
    /// Concrete node implementation, moved into the managed runtime.
    pub node: Box<dyn ProcessNode>,
    /// One entry per input-schema index; `None` = unconnected (dummy port).
    pub inputs: Vec<Option<InputSub>>,
}

/// Builds this node's output subscriber lists plus each port's negotiable
/// protocol capability. `node` must still be locally owned — this is the
/// only point at which the live manager can request capabilities from it
/// (see [`OutputList::capabilities`]'s doc: once `start_node` moves it into
/// its thread, nothing outside that thread can call methods on it again,
/// unlike the offline `Pipeline::build`, which negotiates every connection
/// before any node is spawned). Protocols and payload alternatives come
/// straight off `output_schemas`, which was already obtained from the node
/// without needing a live reference.
fn build_output_lists(
    node: &dyn ProcessNode,
    output_schemas: &[PortSchema],
    input_capabilities: &[Vec<ProtocolCapability>],
) -> Result<HashMap<String, OutputList>, String> {
    let mut outputs: HashMap<String, OutputList> = HashMap::new();
    let registry = TYPE_REGISTRY.lock().unwrap();
    for schema in output_schemas {
        let payloads = schema.payloads.clone();
        let mut lists = Vec::with_capacity(payloads.len());
        for payload in &payloads {
            let sticky = payload.stream_semantics == StreamSemantics::State;
            let list = registry
                .create_shared(payload.type_id, sticky)
                .ok_or_else(|| format!("type of port '{}' not registered", schema.name))?;
            lists.push((payload.type_id, list));
        }

        let mut capabilities = schema
            .protocols
            .iter()
            .copied()
            .filter(|protocol| *protocol != ProtocolKind::Stream)
            .filter_map(|protocol| {
                node.protocol_capability(schema.index, protocol, input_capabilities)
                    .filter(|capability| capability.protocol() == protocol)
            })
            .collect::<Vec<_>>();
        let mut protocols = schema.protocols.clone();
        protocols.retain(|protocol| {
            *protocol == ProtocolKind::Stream
                || capabilities
                    .iter()
                    .any(|capability| capability.protocol() == *protocol)
        });

        outputs.insert(
            schema.name.clone(),
            OutputList {
                payloads,
                lists,
                protocols,
                capabilities: std::mem::take(&mut capabilities),
            },
        );
    }
    Ok(outputs)
}

fn input_protocol_capabilities(
    inputs: &[Option<InputSub>],
    nodes: &HashMap<String, RunningNode>,
) -> Result<Vec<Vec<ProtocolCapability>>, String> {
    inputs
        .iter()
        .map(|input| {
            let Some(input) = input else {
                return Ok(Vec::new());
            };
            let producer = nodes
                .get(&input.from_node)
                .ok_or_else(|| format!("producer '{}' not running", input.from_node))?;
            let output = producer.outputs.get(&input.from_port).ok_or_else(|| {
                format!(
                    "producer '{}' has no port '{}'",
                    input.from_node, input.from_port
                )
            })?;
            Ok(output.capabilities.clone())
        })
        .collect()
}

fn select_node_input_protocols(
    name: &str,
    node: &dyn ProcessNode,
    input_schemas: &[PortSchema],
    inputs: &[Option<InputSub>],
    nodes: &HashMap<String, RunningNode>,
) -> Result<Vec<Option<ProtocolKind>>, String> {
    let mut candidates = vec![None; input_schemas.len()];
    for (index, sub) in inputs.iter().enumerate() {
        let Some(sub) = sub else {
            continue;
        };
        let producer = nodes
            .get(&sub.from_node)
            .ok_or_else(|| format!("producer '{}' not running", sub.from_node))?;
        let output = producer.outputs.get(&sub.from_port).ok_or_else(|| {
            format!(
                "producer '{}' has no port '{}'",
                sub.from_node, sub.from_port
            )
        })?;
        candidates[index] = Some(InputProtocolCandidate {
            offered: output.protocols.clone(),
            capabilities: output.capabilities.clone(),
        });
    }
    let selected = node.select_input_protocols(&candidates);
    if selected.len() != input_schemas.len() {
        return Err(format!(
            "node '{name}' returned {} protocol choices for {} inputs",
            selected.len(),
            input_schemas.len()
        ));
    }
    for (index, candidate) in candidates.iter().enumerate() {
        let Some(candidate) = candidate else {
            continue;
        };
        let protocol = selected[index]
            .ok_or_else(|| format!("no common protocol for node '{name}' input {index}"))?;
        if !candidate.offered.contains(&protocol)
            || !input_schemas[index].protocols.contains(&protocol)
        {
            return Err(format!(
                "node '{name}' selected unsupported protocol {protocol:?} for input {index}"
            ));
        }
    }
    Ok(selected)
}

/// Negotiates one connection's payload representation and returns its list.
fn negotiate_payload_list<'a>(
    output: &'a OutputList,
    accepted: &[PortPayload],
) -> Option<&'a Arc<dyn ErasedSharedSenders>> {
    let negotiated_type = payload_negotiation::negotiate(&output.payloads, accepted)?.type_id;
    Some(
        &output
            .lists
            .iter()
            .find(|(type_id, _)| *type_id == negotiated_type)
            .expect("negotiated type must be one of this output's own lists")
            .1,
    )
}

/// Builds one `OutputPort` from every sender this output actually has —
/// one per negotiated payload representation for a polymorphic port (see
/// [`OutputList::lists`]), folded into a single port the node's `work()`
/// queries by type (`OutputPort::split_senders::<TestValue>()` and
/// `::<TestBlock>()` independently, each seeing only its own senders).
fn output_port_from_lists(output: &OutputList) -> OutputPort {
    let mut port: Option<OutputPort> = None;
    for (type_id, list) in &output.lists {
        let sender = list.sender_box();
        port = Some(match port {
            None => OutputPort::from_type_erased(*type_id, sender),
            Some(p) => p.extend_type_erased(*type_id, sender),
        });
    }
    port.expect("build_output_lists always creates at least one list per port")
}

/// A consumer dropped by [`OverflowPolicy::Disconnect`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectEvent {
    /// Name of the output-owning node.
    pub producer: String,
    /// Producer output port from which the consumer was dropped.
    pub port: String,
    /// Consumer name when it is still managed, otherwise `None`.
    pub consumer: Option<String>,
}

/// Terminal processing error reported by one runtime node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeFailure {
    /// Name of the node whose work loop terminated with an error.
    pub node: String,
    /// Displayable error text reported by that node.
    pub message: String,
}

struct OutputList {
    /// This port's concrete payload alternatives in preference order.
    payloads: Vec<PortPayload>,
    /// One shared subscriber list per concrete `TypeId` this port
    /// actually exposes — one entry for an ordinary port, one per
    /// negotiated kind for a polymorphic TestValue/TestBlock port (e.g. a
    /// raw file channel feeding one `TestValue`-only consumer and one
    /// `TestBlock`-only consumer at once).
    lists: Vec<(TypeId, Arc<dyn ErasedSharedSenders>)>,
    /// Protocols this port can actually deliver.
    protocols: Vec<ProtocolKind>,
    /// Cached output capabilities computed before the node moves into its worker.
    capabilities: Vec<ProtocolCapability>,
}

/// Everything a node needs to run, held between `add_node_deferred` and
/// `start_node`. Deferring the spawn matters at initial materialization:
/// a self-threading source snapshots its subscriber lists on its first
/// `work()`, so every initial consumer must subscribe *before* any thread
/// starts.
struct PendingStart {
    node: Box<dyn ProcessNode>,
    inputs: Vec<InputPort>,
    outputs: Vec<OutputPort>,
    control_rx: crossbeam_channel::Receiver<NodeConfig>,
}

struct RunningNode {
    generation: u64,
    task: Option<Box<dyn WorkTask>>,
    pending: Option<PendingStart>,
    control_tx: crossbeam_channel::Sender<NodeConfig>,
    configuration_scheduler: Option<Arc<dyn ConfigurationScheduler>>,
    cancellation: Option<Arc<dyn super::node::NodeCancellation>>,
    stop_flag: Arc<AtomicBool>,
    /// Set before a restart-kill so the exiting thread does not close the
    /// output lists the replacement will reuse.
    keep_outputs_open: Arc<AtomicBool>,
    /// Items produced across `work()` calls (survives restarts in place).
    items: Arc<AtomicU64>,
    outputs: HashMap<String, OutputList>,
    /// `(producer node, producer port, subscription id)` per connected input.
    input_subs: Vec<(String, String, u64)>,
}

/// Supervisor for a live, threaded graph that may be edited while running.
///
/// Unlike [`Pipeline`](super::pipeline::Pipeline), this manager owns shared subscriber lists so
/// adding, removing, or restarting a node changes only the affected branch.
pub struct PipelineManager {
    nodes: HashMap<String, RunningNode>,
    watchdog: Watchdog,
    watchdog_task: Option<Box<dyn WorkTask>>,
    work_executor: Arc<dyn WorkExecutor>,
    failures: Arc<Mutex<Vec<NodeFailure>>>,
}

impl PipelineManager {
    /// Creates an empty live graph manager and starts its watchdog.
    ///
    /// # Parameters
    /// - `work_executor`: Host capability that runs node and watchdog tasks.
    pub fn new(work_executor: Arc<dyn WorkExecutor>) -> Self {
        let watchdog = Watchdog::new();
        let watchdog_task = watchdog
            .start_monitoring(Arc::clone(&work_executor))
            .expect("host work executor must accept watchdog monitoring");
        Self {
            nodes: HashMap::new(),
            watchdog,
            watchdog_task: Some(watchdog_task),
            work_executor,
            failures: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns the names of nodes currently registered with the manager.
    pub fn node_names(&self) -> Vec<String> {
        self.nodes.keys().cloned().collect()
    }

    /// No-op: threads drive themselves. Exists so callers that hold
    /// [`AppManager`](super::app_manager::AppManager) can call `pump` unconditionally — it only
    /// does real work on [`CooperativeManager`](super::cooperative_manager::CooperativeManager).
    ///
    /// # Parameters
    ///
    /// - `_budget`: Ignored in the threaded runtime; retained for manager API parity.
    pub fn pump(&mut self, _budget: usize) {}

    /// Returns whether a node with `name` is currently registered.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local node name to look up.
    pub fn contains(&self, name: &str) -> bool {
        self.nodes.contains_key(name)
    }

    /// All node threads have exited (run complete or fully stopped).
    pub fn is_finished(&self) -> bool {
        self.nodes.values().all(|node| {
            node.pending.is_none() && node.task.as_ref().is_none_or(|task| task.is_finished())
        })
    }

    /// Adds and starts a node immediately — the live-edit path: producers
    /// are already running, and regular nodes read their subscriber lists
    /// on every send, so a joiner is seen at once.
    ///
    /// # Parameters
    ///
    /// - `spec`: Node implementation and its full input wiring.
    pub fn add_node(&mut self, spec: NodeSpec) -> Result<(), String> {
        let name = spec.name.clone();
        self.add_node_deferred(spec)?;
        self.start_node(&name)
    }

    /// Registers a node — lists created, inputs subscribed — without
    /// starting its thread. Initial materialization adds every node
    /// deferred, then calls [`Self::start_all_deferred`], so no producer
    /// can run ahead of (or snapshot past) its initial consumers.
    ///
    /// # Parameters
    ///
    /// - `spec`: Node implementation and its full input wiring.
    pub fn add_node_deferred(&mut self, spec: NodeSpec) -> Result<(), String> {
        if self.nodes.contains_key(&spec.name) {
            return Err(format!("node '{}' already exists", spec.name));
        }
        let NodeSpec { name, node, inputs } = spec;

        let input_schemas = node.input_schema();
        let output_schemas = node.output_schema();
        if inputs.len() != input_schemas.len() {
            return Err(format!(
                "node '{}': {} input specs for {} ports",
                name,
                inputs.len(),
                input_schemas.len()
            ));
        }

        // Output subscriber lists, supervisor-owned. `node` is still
        // locally owned here (not yet moved into a thread), so this is the
        // only point where its output capabilities can be requested.
        let input_capabilities = input_protocol_capabilities(&inputs, &self.nodes)?;
        let outputs = build_output_lists(node.as_ref(), &output_schemas, &input_capabilities)?;
        let selected_protocols = select_node_input_protocols(
            &name,
            node.as_ref(),
            &input_schemas,
            &inputs,
            &self.nodes,
        )?;

        // Wire inputs: subscribe into the producers' lists, unless this
        // connection negotiates a capability (producer has a cached handle for
        // that port *and* this node accepts it), in which case no stream
        // subscription happens at all.
        let mut input_ports: Vec<InputPort> = Vec::with_capacity(inputs.len());
        let mut input_subs: Vec<(String, String, u64)> = Vec::new();
        for (index, sub) in inputs.iter().enumerate() {
            let port = match sub {
                None => InputPort::from_type_erased(Box::new(()) as Box<dyn Any + Send>),
                Some(sub) => {
                    let producer = self
                        .nodes
                        .get(&sub.from_node)
                        .ok_or_else(|| format!("producer '{}' not running", sub.from_node))?;
                    let output = producer.outputs.get(&sub.from_port).ok_or_else(|| {
                        format!(
                            "producer '{}' has no port '{}'",
                            sub.from_node, sub.from_port
                        )
                    })?;
                    let list = negotiate_payload_list(output, &input_schemas[index].payloads)
                        .ok_or_else(|| {
                            format!(
                                "type mismatch: {}.{} -> {}.{}",
                                sub.from_node, sub.from_port, name, input_schemas[index].name
                            )
                        })?;
                    if let Some(protocol @ ProtocolKind::Capability(_)) = selected_protocols[index]
                    {
                        let capability = output
                            .capabilities
                            .iter()
                            .find(|capability| capability.protocol() == protocol)
                            .cloned()
                            .ok_or_else(|| {
                                format!(
                                    "producer '{}.{}' has no {protocol:?} capability",
                                    sub.from_node, sub.from_port
                                )
                            })?;
                        InputPort::from_type_erased(Box::new(()) as Box<dyn Any + Send>)
                            .with_protocol_capability(Some(capability))
                    } else {
                        let label = Some(format!("{}.{}", name, input_schemas[index].name));
                        let subscription = list.subscribe_with_label(sub.buffer, sub.policy, label);
                        input_subs.push((
                            sub.from_node.clone(),
                            sub.from_port.clone(),
                            subscription.id,
                        ));
                        InputPort::from_type_erased(subscription.receiver)
                            .with_available_protocol_capabilities(output.capabilities.clone())
                    }
                }
            };
            let port_name = input_schemas
                .get(index)
                .map(|schema| schema.name.clone())
                .unwrap_or_else(|| format!("in{index}"));
            input_ports.push(port.with_watchdog(self.watchdog.clone(), name.clone(), port_name));
        }

        let output_ports: Vec<OutputPort> = output_schemas
            .iter()
            .map(|schema| {
                output_port_from_lists(&outputs[&schema.name]).with_watchdog(
                    self.watchdog.clone(),
                    name.clone(),
                    schema.name.clone(),
                )
            })
            .collect();

        self.register(
            name,
            node,
            input_ports,
            output_ports,
            outputs,
            input_subs,
            0,
        );
        Ok(())
    }

    /// Stores a fully wired node awaiting `start_node`.
    #[allow(clippy::too_many_arguments)]
    fn register(
        &mut self,
        name: String,
        node: Box<dyn ProcessNode>,
        inputs: Vec<InputPort>,
        outputs: Vec<OutputPort>,
        output_lists: HashMap<String, OutputList>,
        input_subs: Vec<(String, String, u64)>,
        generation: u64,
    ) {
        let configuration_scheduler = node.configuration_scheduler();
        let cancellation = node.cancellation();
        let (control_tx, control_rx) = crossbeam_channel::unbounded::<NodeConfig>();
        let items = Arc::new(AtomicU64::new(0));
        self.nodes.insert(
            name,
            RunningNode {
                generation,
                task: None,
                pending: Some(PendingStart {
                    node,
                    inputs,
                    outputs,
                    control_rx,
                }),
                control_tx,
                configuration_scheduler,
                cancellation,
                stop_flag: Arc::new(AtomicBool::new(false)),
                keep_outputs_open: Arc::new(AtomicBool::new(false)),
                items,
                outputs: output_lists,
                input_subs,
            },
        );
    }

    /// Starts every node still awaiting its thread (initial bring-up).
    pub fn start_all_deferred(&mut self) -> Result<(), String> {
        let deferred: Vec<String> = self
            .nodes
            .iter()
            .filter(|(_, node)| node.pending.is_some())
            .map(|(name, _)| name.clone())
            .collect();
        for name in deferred {
            self.start_node(&name)?;
        }
        Ok(())
    }

    fn start_node(&mut self, name: &str) -> Result<(), String> {
        let work_executor = Arc::clone(&self.work_executor);
        let entry = self
            .nodes
            .get_mut(name)
            .ok_or_else(|| format!("node '{name}' not registered"))?;
        let Some(PendingStart {
            mut node,
            inputs,
            outputs,
            control_rx,
        }) = entry.pending.take()
        else {
            return Err(format!("node '{name}' already started"));
        };
        let generation = entry.generation;

        let thread_name = format!("{name}@{generation}");
        let thread_stop = Arc::clone(&entry.stop_flag);
        let thread_keep_open = Arc::clone(&entry.keep_outputs_open);
        let thread_items = Arc::clone(&entry.items);
        let failures = Arc::clone(&self.failures);
        let failure_node = name.to_owned();
        let close_handles: Vec<Arc<dyn ErasedSharedSenders>> = entry
            .outputs
            .values()
            .flat_map(|output| output.lists.iter().map(|(_, list)| Arc::clone(list)))
            .collect();

        let task_executor = Arc::clone(&work_executor);
        let task = work_executor
            .submit_long_running(Box::new(move || {
                if node.is_self_threading() {
                    // Start internal threads once, then supervise.
                    if let Err(e) = node.work_outcome(&inputs, &outputs) {
                        error!("[{thread_name}] failed to start: {e}");
                        failures.lock().unwrap().push(NodeFailure {
                            node: failure_node.clone(),
                            message: e.to_string(),
                        });
                    } else {
                        loop {
                            if thread_stop.load(Ordering::Relaxed) || node.should_stop() {
                                break;
                            }
                            while let Ok(config) = control_rx.try_recv() {
                                if node.apply_config(&config) == ConfigOutcome::NeedsRestart {
                                    error!("[{thread_name}] config not hot-appliable");
                                }
                            }
                            task_executor.idle(std::time::Duration::from_millis(50));
                        }
                    }
                } else {
                    let mut idle_rounds = 0_u8;
                    loop {
                        while let Ok(config) = control_rx.try_recv() {
                            if node.apply_config(&config) == ConfigOutcome::NeedsRestart {
                                error!("[{thread_name}] config not hot-appliable");
                            }
                        }
                        if thread_stop.load(Ordering::Relaxed) || node.should_stop() {
                            break;
                        }
                        match node.work_outcome(&inputs, &outputs) {
                            Ok(outcome) => {
                                if outcome.produced_items() > 0 {
                                    thread_items.fetch_add(
                                        outcome.produced_items() as u64,
                                        Ordering::Relaxed,
                                    );
                                }
                                if outcome.made_progress() {
                                    idle_rounds = 0;
                                } else if idle_rounds < 16 {
                                    idle_rounds += 1;
                                    task_executor.idle(std::time::Duration::ZERO);
                                } else {
                                    task_executor.idle(std::time::Duration::from_micros(50));
                                }
                            }
                            Err(WorkError::Shutdown) => {
                                debug!("[{thread_name}] shutdown");
                                break;
                            }
                            Err(e) => {
                                error!("[{thread_name}] work error: {e}");
                                failures.lock().unwrap().push(NodeFailure {
                                    node: failure_node.clone(),
                                    message: e.to_string(),
                                });
                                break;
                            }
                        }
                    }
                }

                // Flush/close node resources (writer Drop, source threads).
                drop(node);
                drop(inputs);
                drop(outputs);

                // Natural completion propagates EOS downstream. A restart
                // kill keeps the lists open for the replacement instance.
                if !thread_keep_open.load(Ordering::Relaxed) {
                    for list in &close_handles {
                        list.close();
                    }
                }
                info!("[{thread_name}] exited");
            }))
            .map_err(|error| format!("start '{name}': {error}"))?;

        entry.task = Some(task);
        Ok(())
    }

    /// Unsubscribes `name` from its producers (its next reads see
    /// end-of-stream), closes its own output lists (the cascade continues
    /// through the branch), and joins the thread.
    ///
    /// # Parameters
    /// - `name`: Name of the node to detach, stop, and join.
    pub fn remove_node(&mut self, name: &str) -> Result<(), String> {
        let node = self
            .nodes
            .remove(name)
            .ok_or_else(|| format!("node '{name}' not running"))?;
        self.detach(&node);
        node.stop_flag.store(true, Ordering::Relaxed);
        if let Some(cancellation) = &node.cancellation {
            cancellation.request_cancel();
        }
        for output in node.outputs.values() {
            for (_, list) in &output.lists {
                list.close();
            }
        }
        if let Some(task) = node.task {
            task.wait();
        }
        Ok(())
    }

    fn detach(&self, node: &RunningNode) {
        for (from_node, from_port, sub_id) in &node.input_subs {
            if let Some(producer) = self.nodes.get(from_node)
                && let Some(output) = producer.outputs.get(from_port)
            {
                // Subscription ids are globally unique (one counter shared
                // across every SharedSenders list, see sender.rs), so
                // unsubscribing from a list that doesn't hold this id is a
                // harmless no-op — no need to track which negotiated kind
                // this particular subscription resolved to.
                for (_, list) in &output.lists {
                    list.unsubscribe(*sub_id);
                }
            }
        }
    }

    /// Sends a hot configuration to a running node; applied between
    /// `work()` calls. Whether the change is hot-appliable is decided
    /// statically by the caller (builder capability), so a `NeedsRestart`
    /// outcome inside the node is logged as a bug rather than handled.
    ///
    /// # Parameters
    ///
    /// - `name`: Name of the running node to configure.
    /// - `config`: Validated configuration to apply between work calls.
    pub fn reconfigure(&self, name: &str, config: NodeConfig) -> Result<(), String> {
        let node = self
            .nodes
            .get(name)
            .ok_or_else(|| format!("node '{name}' not running"))?;
        node.control_tx
            .send(config)
            .map_err(|_| format!("node '{name}' no longer accepts config"))
    }

    /// Schedules validated hot configuration at an event-time boundary.
    /// The node, rather than its worker loop, decides when the boundary is
    /// crossed so already queued older input retains the prior settings.
    ///
    /// # Parameters
    ///
    /// - `name`: Name of the running node to configure.
    /// - `config`: Validated configuration to apply.
    /// - `boundary`: Event-time boundary at which the node should activate it.
    pub fn reconfigure_at(
        &self,
        name: &str,
        config: NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> Result<(), String> {
        let node = self
            .nodes
            .get(name)
            .ok_or_else(|| format!("node '{name}' not running"))?;
        let scheduler = node.configuration_scheduler.as_ref().ok_or_else(|| {
            format!("node '{name}' does not expose a scheduled configuration handle")
        })?;
        if scheduler.schedule_config(&config, boundary) == ConfigOutcome::NeedsRestart {
            return Err(format!(
                "node '{name}' rejected scheduled hot configuration"
            ));
        }
        Ok(())
    }

    /// Replaces a running node with a fresh instance wired to the *same*
    /// output lists (downstream connections survive untouched), generation
    /// +1. `inputs` re-declares its input wiring.
    ///
    /// # Parameters
    /// - `name`: Name of the logical node to replace.
    /// - `node`: Fresh implementation instance.
    /// - `inputs`: Replacement input wiring in input-schema order.
    pub fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn ProcessNode>,
        inputs: Vec<Option<InputSub>>,
    ) -> Result<(), String> {
        let old = self
            .nodes
            .remove(name)
            .ok_or_else(|| format!("node '{name}' not running"))?;
        old.keep_outputs_open.store(true, Ordering::Relaxed);
        self.detach(&old);
        old.stop_flag.store(true, Ordering::Relaxed);
        if let Some(cancellation) = &old.cancellation {
            cancellation.request_cancel();
        }
        if let Some(task) = old.task {
            task.wait();
        }

        let input_schemas = node.input_schema();
        if inputs.len() != input_schemas.len() {
            return Err(format!(
                "node '{name}': {} input specs for {} ports",
                inputs.len(),
                input_schemas.len()
            ));
        }
        let selected_protocols =
            select_node_input_protocols(name, node.as_ref(), &input_schemas, &inputs, &self.nodes)?;
        let mut input_ports: Vec<InputPort> = Vec::with_capacity(inputs.len());
        let mut input_subs: Vec<(String, String, u64)> = Vec::new();
        for (index, sub) in inputs.iter().enumerate() {
            let port = match sub {
                None => InputPort::from_type_erased(Box::new(()) as Box<dyn Any + Send>),
                Some(sub) => {
                    let producer = self
                        .nodes
                        .get(&sub.from_node)
                        .ok_or_else(|| format!("producer '{}' not running", sub.from_node))?;
                    let output = producer.outputs.get(&sub.from_port).ok_or_else(|| {
                        format!(
                            "producer '{}' has no port '{}'",
                            sub.from_node, sub.from_port
                        )
                    })?;
                    let list = negotiate_payload_list(output, &input_schemas[index].payloads)
                        .ok_or_else(|| {
                            format!(
                                "type mismatch: {}.{} -> {}.{}",
                                sub.from_node, sub.from_port, name, input_schemas[index].name
                            )
                        })?;
                    if let Some(protocol @ ProtocolKind::Capability(_)) = selected_protocols[index]
                    {
                        let capability = output
                            .capabilities
                            .iter()
                            .find(|capability| capability.protocol() == protocol)
                            .cloned()
                            .ok_or_else(|| {
                                format!(
                                    "producer '{}.{}' has no {protocol:?} capability",
                                    sub.from_node, sub.from_port
                                )
                            })?;
                        InputPort::from_type_erased(Box::new(()) as Box<dyn Any + Send>)
                            .with_protocol_capability(Some(capability))
                    } else {
                        let label = Some(format!("{}.{}", name, input_schemas[index].name));
                        let subscription = list.subscribe_with_label(sub.buffer, sub.policy, label);
                        input_subs.push((
                            sub.from_node.clone(),
                            sub.from_port.clone(),
                            subscription.id,
                        ));
                        InputPort::from_type_erased(subscription.receiver)
                            .with_available_protocol_capabilities(output.capabilities.clone())
                    }
                }
            };
            let port_name = input_schemas
                .get(index)
                .map(|schema| schema.name.clone())
                .unwrap_or_else(|| format!("in{index}"));
            input_ports.push(port.with_watchdog(self.watchdog.clone(), name.to_owned(), port_name));
        }

        let output_schemas = node.output_schema();
        let output_ports: Vec<OutputPort> = output_schemas
            .iter()
            .map(|schema| {
                output_port_from_lists(&old.outputs[&schema.name]).with_watchdog(
                    self.watchdog.clone(),
                    name.to_owned(),
                    schema.name.clone(),
                )
            })
            .collect();

        let generation = old.generation + 1;
        self.register(
            name.to_owned(),
            node,
            input_ports,
            output_ports,
            old.outputs,
            input_subs,
            generation,
        );
        // Same logical node: the progress count carries across the restart.
        if let Some(entry) = self.nodes.get_mut(name) {
            entry.items = Arc::clone(&old.items);
        }
        self.start_node(name)
    }

    /// Items produced per node (sum of `work()` return values), for
    /// progress display. Self-threading sources report 0.
    pub fn progress(&self) -> Vec<(String, u64)> {
        self.nodes
            .iter()
            .map(|(name, node)| (name.clone(), node.items.load(Ordering::Relaxed)))
            .collect()
    }

    /// Consumers dropped by `OverflowPolicy::Disconnect` since the last call.
    pub fn take_disconnected(&self) -> Vec<DisconnectEvent> {
        // Reverse map: subscription id → consumer node.
        let mut consumers: HashMap<u64, &str> = HashMap::new();
        for (name, node) in &self.nodes {
            for (_, _, sub_id) in &node.input_subs {
                consumers.insert(*sub_id, name);
            }
        }
        let mut events = Vec::new();
        for (name, node) in &self.nodes {
            for (port, output) in &node.outputs {
                for (_, list) in &output.lists {
                    for sub_id in list.take_disconnected() {
                        events.push(DisconnectEvent {
                            producer: name.clone(),
                            port: port.clone(),
                            consumer: consumers.get(&sub_id).map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
        events
    }

    /// Returns and clears all terminal node failures observed since the last call.
    pub fn take_failures(&mut self) -> Vec<NodeFailure> {
        std::mem::take(&mut *self.failures.lock().unwrap())
    }

    /// Signals every node to stop — sets all stop flags and closes every
    /// output list (unblocking every blocked send/recv with end-of-stream)
    /// — **without joining the threads**. This is the stop an interactive
    /// caller must use: [`Self::stop_all`]'s joins wait for each node to
    /// finish its current `work()` call, which a frame loop can never
    /// afford to block on. Poll [`Self::is_finished`] to observe the
    /// wind-down; the exited threads are reaped by a later [`Self::stop_all`]
    /// (instant once finished) or by drop. Idempotent.
    pub fn request_stop(&self) {
        for node in self.nodes.values() {
            node.stop_flag.store(true, Ordering::Relaxed);
            if let Some(cancellation) = &node.cancellation {
                cancellation.request_cancel();
            }
            for output in node.outputs.values() {
                for (_, list) in &output.lists {
                    list.close();
                }
            }
        }
    }

    /// Stops everything: closes every output list (unblocking every waiting
    /// consumer with end-of-stream), sets all stop flags, joins all threads.
    /// Writer flushes run in the node `Drop`s, as offline.
    pub fn stop_all(&mut self) {
        self.request_stop();
        for (_, node) in self.nodes.drain() {
            if let Some(task) = node.task {
                task.wait();
            }
        }
        self.watchdog.stop();
        if let Some(task) = self.watchdog_task.take() {
            task.wait();
        }
    }

    /// Blocks until every node thread has exited (natural end of a file
    /// run), then reaps them. Live edits must come from another thread's
    /// point of view *before* calling this.
    pub fn wait(&mut self) {
        for (_, node) in self.nodes.drain() {
            if let Some(task) = node.task {
                task.wait();
            }
        }
        self.watchdog.stop();
        if let Some(task) = self.watchdog_task.take() {
            task.wait();
        }
    }
}

impl Drop for PipelineManager {
    fn drop(&mut self) {
        if !self.nodes.is_empty() {
            self.stop_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use platform_runtime::{WorkExecutor, WorkExecutorTask, WorkTask};

    use super::super::errors::WorkResult;
    use super::super::node::{ConfigValue, WorkOutcome};
    use super::super::ports::{PortDirection, PortSchema};
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TestLevel {
        value: i64,
        start_time_ns: u64,
    }

    impl TestLevel {
        fn new(value: i64, start_time_ns: u64) -> Self {
            Self {
                value,
                start_time_ns,
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TestValue {
        value: bool,
        start_time_ns: u64,
    }

    impl TestValue {
        fn new(value: bool, start_time_ns: u64) -> Self {
            Self {
                value,
                start_time_ns,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct TestBlock {
        data: Arc<[u8]>,
        start: u64,
        len: usize,
        step: u64,
    }

    impl TestBlock {
        fn new(data: Arc<[u8]>, start: u64, len: usize, step: u64) -> Self {
            Self {
                data,
                start,
                len,
                step,
            }
        }
    }

    struct TestWorkExecutor;

    impl WorkExecutor for TestWorkExecutor {
        fn available_parallelism(&self) -> usize {
            2
        }

        fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
            Ok(Box::new(TestWorkTask {
                handle: Some(std::thread::spawn(task)),
            }))
        }
    }

    struct IdleRecordingExecutor {
        idle_calls: Arc<AtomicU64>,
    }

    impl WorkExecutor for IdleRecordingExecutor {
        fn available_parallelism(&self) -> usize {
            2
        }

        fn idle(&self, duration: Duration) {
            if duration <= Duration::from_micros(50) {
                self.idle_calls.fetch_add(1, Ordering::Relaxed);
            }
        }

        fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
            Ok(Box::new(TestWorkTask {
                handle: Some(std::thread::spawn(task)),
            }))
        }
    }

    struct TestWorkTask {
        handle: Option<JoinHandle<()>>,
    }

    impl WorkTask for TestWorkTask {
        fn is_finished(&self) -> bool {
            self.handle.as_ref().is_none_or(JoinHandle::is_finished)
        }

        fn wait(mut self: Box<Self>) {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn manager() -> PipelineManager {
        PipelineManager::new(Arc::new(TestWorkExecutor))
    }

    /// Emits `TestLevel { value: i, start_time_ns: i }` for i in 0..max,
    /// paced so tests can attach taps mid-stream.
    struct PacedSource {
        next: i64,
        max: i64,
        pace: Duration,
    }

    struct BlockingCancellation {
        sender: crossbeam_channel::Sender<()>,
    }

    impl super::super::node::NodeCancellation for BlockingCancellation {
        fn request_cancel(&self) {
            let _ = self.sender.try_send(());
        }
    }

    struct BlockingCancelableNode {
        receiver: crossbeam_channel::Receiver<()>,
        cancellation: Arc<BlockingCancellation>,
    }

    struct FailingNode;

    struct ZeroOutputProgressNode {
        remaining: usize,
    }

    impl ProcessNode for ZeroOutputProgressNode {
        fn name(&self) -> &str {
            "zero_output_progress"
        }

        fn should_stop(&self) -> bool {
            self.remaining == 0
        }

        fn num_inputs(&self) -> usize {
            0
        }

        fn num_outputs(&self) -> usize {
            0
        }

        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            unreachable!("the test overrides work_outcome")
        }

        fn work_outcome(
            &mut self,
            _inputs: &[InputPort],
            _outputs: &[OutputPort],
        ) -> WorkResult<WorkOutcome> {
            self.remaining = self.remaining.saturating_sub(1);
            Ok(WorkOutcome::progressed(0))
        }
    }

    impl ProcessNode for FailingNode {
        fn name(&self) -> &str {
            "failing"
        }

        fn num_inputs(&self) -> usize {
            0
        }

        fn num_outputs(&self) -> usize {
            0
        }

        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            Err(WorkError::NodeError("intentional failure".into()))
        }
    }

    impl ProcessNode for BlockingCancelableNode {
        fn name(&self) -> &str {
            "blocking_cancelable"
        }

        fn num_inputs(&self) -> usize {
            0
        }

        fn num_outputs(&self) -> usize {
            0
        }

        fn cancellation(&self) -> Option<Arc<dyn super::super::node::NodeCancellation>> {
            Some(self.cancellation.clone())
        }

        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            let _ = self.receiver.recv();
            Err(WorkError::Shutdown)
        }
    }

    impl ProcessNode for PacedSource {
        fn name(&self) -> &str {
            "paced_source"
        }
        fn num_inputs(&self) -> usize {
            0
        }
        fn num_outputs(&self) -> usize {
            1
        }
        fn output_schema(&self) -> Vec<PortSchema> {
            vec![PortSchema::state::<TestLevel>(
                "out",
                0,
                PortDirection::Output,
            )]
        }
        fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
            if self.next >= self.max {
                return Err(WorkError::Shutdown);
            }
            let output = outputs[0]
                .get::<TestLevel>()
                .ok_or_else(|| WorkError::NodeError("missing output".into()))?;
            output.send(TestLevel {
                value: self.next,
                start_time_ns: self.next as u64,
            })?;
            self.next += 1;
            std::thread::sleep(self.pace);
            Ok(1)
        }
    }

    struct ControlledSource {
        receiver: crossbeam_channel::Receiver<TestLevel>,
    }

    impl ProcessNode for ControlledSource {
        fn name(&self) -> &str {
            "controlled_source"
        }
        fn num_inputs(&self) -> usize {
            0
        }
        fn num_outputs(&self) -> usize {
            1
        }
        fn output_schema(&self) -> Vec<PortSchema> {
            vec![PortSchema::state::<TestLevel>(
                "out",
                0,
                PortDirection::Output,
            )]
        }
        fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
            let sample = self.receiver.recv().map_err(|_| WorkError::Shutdown)?;
            let output = outputs[0]
                .get::<TestLevel>()
                .ok_or_else(|| WorkError::NodeError("missing output".into()))?;
            output.send(sample)?;
            Ok(1)
        }
    }

    /// Adds a configurable offset; hot-appliable.
    struct AddOffset {
        offset: i64,
        buffer: VecDeque<TestLevel>,
        scheduled: Arc<Mutex<VecDeque<(ConfigurationBoundary, i64)>>>,
    }

    struct OffsetConfigurationScheduler {
        scheduled: Arc<Mutex<VecDeque<(ConfigurationBoundary, i64)>>>,
    }

    impl ConfigurationScheduler for OffsetConfigurationScheduler {
        fn schedule_config(
            &self,
            config: &NodeConfig,
            boundary: ConfigurationBoundary,
        ) -> ConfigOutcome {
            let Some(ConfigValue::I64(offset)) = config.get("offset") else {
                return ConfigOutcome::NeedsRestart;
            };
            self.scheduled
                .lock()
                .unwrap()
                .push_back((boundary, *offset));
            ConfigOutcome::Applied
        }
    }

    impl ProcessNode for AddOffset {
        fn name(&self) -> &str {
            "add_offset"
        }
        fn num_inputs(&self) -> usize {
            1
        }
        fn num_outputs(&self) -> usize {
            1
        }
        fn input_schema(&self) -> Vec<PortSchema> {
            vec![PortSchema::state::<TestLevel>(
                "in",
                0,
                PortDirection::Input,
            )]
        }
        fn output_schema(&self) -> Vec<PortSchema> {
            vec![PortSchema::state::<TestLevel>(
                "out",
                0,
                PortDirection::Output,
            )]
        }
        fn apply_config(&mut self, config: &NodeConfig) -> ConfigOutcome {
            if let Some(ConfigValue::I64(offset)) = config.get("offset") {
                self.offset = *offset;
                ConfigOutcome::Applied
            } else {
                ConfigOutcome::NeedsRestart
            }
        }
        fn configuration_scheduler(&self) -> Option<Arc<dyn ConfigurationScheduler>> {
            Some(Arc::new(OffsetConfigurationScheduler {
                scheduled: Arc::clone(&self.scheduled),
            }))
        }
        fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
            let mut input = inputs[0]
                .get::<TestLevel>(&mut self.buffer)
                .ok_or_else(|| WorkError::NodeError("missing input".into()))?;
            let sample = input.recv()?;
            let mut scheduled = self.scheduled.lock().unwrap();
            while scheduled
                .front()
                .is_some_and(|(boundary, _)| boundary.timestamp_ns <= sample.start_time_ns)
            {
                self.offset = scheduled.pop_front().unwrap().1;
            }
            drop(scheduled);
            let output = outputs[0]
                .get::<TestLevel>()
                .ok_or_else(|| WorkError::NodeError("missing output".into()))?;
            output.send(TestLevel {
                value: sample.value + self.offset,
                start_time_ns: sample.start_time_ns,
            })?;
            Ok(1)
        }
    }

    struct Collect {
        store: Arc<Mutex<Vec<i64>>>,
        buffer: VecDeque<TestLevel>,
    }

    impl ProcessNode for Collect {
        fn name(&self) -> &str {
            "collect"
        }
        fn num_inputs(&self) -> usize {
            1
        }
        fn num_outputs(&self) -> usize {
            0
        }
        fn input_schema(&self) -> Vec<PortSchema> {
            vec![PortSchema::state::<TestLevel>(
                "in",
                0,
                PortDirection::Input,
            )]
        }
        fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            let mut input = inputs[0]
                .get::<TestLevel>(&mut self.buffer)
                .ok_or_else(|| WorkError::NodeError("missing input".into()))?;
            let sample = input.recv()?;
            self.store.lock().unwrap().push(sample.value);
            Ok(1)
        }
    }

    fn sub(from: &str, port: &str) -> Option<InputSub> {
        Some(InputSub {
            from_node: from.to_owned(),
            from_port: port.to_owned(),
            buffer: 64,
            policy: OverflowPolicy::Block,
        })
    }

    fn collect_spec(name: &str, from: &str, port: &str, store: &Arc<Mutex<Vec<i64>>>) -> NodeSpec {
        NodeSpec {
            name: name.to_owned(),
            node: Box::new(Collect {
                store: Arc::clone(store),
                buffer: VecDeque::new(),
            }),
            inputs: vec![sub(from, port)],
        }
    }

    fn wait_finished(manager: &PipelineManager, timeout: Duration) {
        let start = std::time::Instant::now();
        while !manager.is_finished() {
            assert!(start.elapsed() < timeout, "pipeline did not finish in time");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn zero_output_progress_does_not_enter_idle_backoff() {
        let idle_calls = Arc::new(AtomicU64::new(0));
        let mut manager = PipelineManager::new(Arc::new(IdleRecordingExecutor {
            idle_calls: Arc::clone(&idle_calls),
        }));
        manager
            .add_node(NodeSpec {
                name: "zero-output-progress".into(),
                node: Box::new(ZeroOutputProgressNode { remaining: 64 }),
                inputs: Vec::new(),
            })
            .unwrap();

        wait_finished(&manager, Duration::from_secs(1));
        manager.wait();

        assert_eq!(idle_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn terminal_node_errors_are_observable_by_the_runtime_owner() {
        let mut manager = manager();
        manager
            .add_node(NodeSpec {
                name: "failure-node".into(),
                node: Box::new(FailingNode),
                inputs: Vec::new(),
            })
            .unwrap();

        wait_finished(&manager, Duration::from_secs(1));

        assert_eq!(
            manager.take_failures(),
            vec![NodeFailure {
                node: "failure-node".into(),
                message: "Node-specific error: intentional failure".into(),
            }]
        );
        assert!(manager.take_failures().is_empty());
    }

    #[test]
    fn add_tap_mid_run_gets_sticky_prime_and_live_data() {
        let mut manager = manager();
        let base = Arc::new(Mutex::new(Vec::new()));
        let tap = Arc::new(Mutex::new(Vec::new()));

        manager
            .add_node(NodeSpec {
                name: "source".into(),
                node: Box::new(PacedSource {
                    next: 0,
                    max: 100,
                    pace: Duration::from_millis(2),
                }),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node(collect_spec("base", "source", "out", &base))
            .unwrap();

        // Let some values flow, then attach the tap.
        std::thread::sleep(Duration::from_millis(60));
        manager
            .add_node(collect_spec("tap", "source", "out", &tap))
            .unwrap();

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();

        let base = base.lock().unwrap();
        let tap = tap.lock().unwrap();
        assert_eq!(base.as_slice(), (0..100).collect::<Vec<i64>>().as_slice());
        assert!(!tap.is_empty(), "tap received nothing");
        let first = tap[0];
        assert!(first > 0, "tap joined mid-stream, got {first}");
        // Sticky priming: the first value is the level current at join time,
        // then the stream continues gapless.
        assert_eq!(
            tap.as_slice(),
            (first..100).collect::<Vec<i64>>().as_slice(),
            "tap stream has gaps"
        );
    }

    #[test]
    fn remove_branch_leaves_the_rest_running() {
        let mut manager = manager();
        let base = Arc::new(Mutex::new(Vec::new()));
        let doomed = Arc::new(Mutex::new(Vec::new()));

        manager
            .add_node(NodeSpec {
                name: "source".into(),
                node: Box::new(PacedSource {
                    next: 0,
                    max: 100,
                    pace: Duration::from_millis(2),
                }),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node(collect_spec("base", "source", "out", &base))
            .unwrap();
        manager
            .add_node(collect_spec("doomed", "source", "out", &doomed))
            .unwrap();

        std::thread::sleep(Duration::from_millis(50));
        manager.remove_node("doomed").unwrap();
        let doomed_count = doomed.lock().unwrap().len();
        assert!(doomed_count > 0, "branch never received data");

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();
        assert_eq!(base.lock().unwrap().len(), 100, "survivor lost data");
        assert!(
            doomed.lock().unwrap().len() < 100,
            "removed branch kept receiving"
        );
    }

    #[test]
    fn reconfigure_applies_between_work_calls() {
        let mut manager = manager();
        let out = Arc::new(Mutex::new(Vec::new()));

        manager
            .add_node(NodeSpec {
                name: "source".into(),
                node: Box::new(PacedSource {
                    next: 0,
                    max: 60,
                    pace: Duration::from_millis(2),
                }),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node(NodeSpec {
                name: "offset".into(),
                node: Box::new(AddOffset {
                    offset: 0,
                    buffer: VecDeque::new(),
                    scheduled: Arc::new(Mutex::new(VecDeque::new())),
                }),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();
        manager
            .add_node(collect_spec("sink", "offset", "out", &out))
            .unwrap();

        std::thread::sleep(Duration::from_millis(40));
        let mut config = NodeConfig::new();
        config.insert("offset".into(), ConfigValue::I64(1000));
        manager.reconfigure("offset", config).unwrap();

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();

        let values = out.lock().unwrap();
        assert_eq!(values.len(), 60);
        assert!(
            values.first().copied().unwrap() < 1000,
            "config applied too early?"
        );
        assert!(
            values.last().copied().unwrap() >= 1000,
            "config never applied"
        );
        // Offset flips exactly once: values are (i) then (i + 1000), both
        // strictly increasing.
        let flips = values.windows(2).filter(|w| w[1] < w[0]).count();
        assert_eq!(flips, 0, "stream went backwards: {values:?}");
    }

    #[test]
    fn scheduled_reconfigure_switches_at_event_boundary_with_queued_input() {
        let mut manager = manager();
        let out = Arc::new(Mutex::new(Vec::new()));
        manager
            .add_node_deferred(NodeSpec {
                name: "source".into(),
                node: Box::new(PacedSource {
                    next: 0,
                    max: 60,
                    pace: Duration::ZERO,
                }),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node_deferred(NodeSpec {
                name: "offset".into(),
                node: Box::new(AddOffset {
                    offset: 0,
                    buffer: VecDeque::new(),
                    scheduled: Arc::new(Mutex::new(VecDeque::new())),
                }),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();
        manager
            .add_node_deferred(collect_spec("sink", "offset", "out", &out))
            .unwrap();
        manager
            .reconfigure_at(
                "offset",
                NodeConfig::from([("offset".into(), ConfigValue::I64(1000))]),
                ConfigurationBoundary::new(40, 40),
            )
            .unwrap();
        manager.start_all_deferred().unwrap();
        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();

        let values = out.lock().unwrap();
        assert_eq!(&values[..40], (0..40).collect::<Vec<_>>().as_slice());
        assert_eq!(&values[40..], (1040..1060).collect::<Vec<_>>().as_slice());
    }

    #[test]
    fn scheduled_reconfigure_reaches_node_while_work_is_blocked_for_input() {
        let mut manager = manager();
        let out = Arc::new(Mutex::new(Vec::new()));
        let (source_sender, source_receiver) = crossbeam_channel::unbounded();
        manager
            .add_node_deferred(NodeSpec {
                name: "source".into(),
                node: Box::new(ControlledSource {
                    receiver: source_receiver,
                }),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node_deferred(NodeSpec {
                name: "offset".into(),
                node: Box::new(AddOffset {
                    offset: 0,
                    buffer: VecDeque::new(),
                    scheduled: Arc::new(Mutex::new(VecDeque::new())),
                }),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();
        manager
            .add_node_deferred(collect_spec("sink", "offset", "out", &out))
            .unwrap();
        manager.start_all_deferred().unwrap();
        std::thread::sleep(Duration::from_millis(20));

        manager
            .reconfigure_at(
                "offset",
                NodeConfig::from([("offset".into(), ConfigValue::I64(1000))]),
                ConfigurationBoundary::new(40, 40),
            )
            .unwrap();
        source_sender.send(TestLevel::new(40, 40)).unwrap();
        drop(source_sender);
        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();

        assert_eq!(out.lock().unwrap().as_slice(), &[1040]);
    }

    #[test]
    fn restart_in_place_keeps_downstream_attached() {
        let mut manager = manager();
        let out = Arc::new(Mutex::new(Vec::new()));

        manager
            .add_node(NodeSpec {
                name: "source".into(),
                node: Box::new(PacedSource {
                    next: 0,
                    max: 100,
                    pace: Duration::from_millis(2),
                }),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node(NodeSpec {
                name: "offset".into(),
                node: Box::new(AddOffset {
                    offset: 0,
                    buffer: VecDeque::new(),
                    scheduled: Arc::new(Mutex::new(VecDeque::new())),
                }),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();
        manager
            .add_node(collect_spec("sink", "offset", "out", &out))
            .unwrap();

        std::thread::sleep(Duration::from_millis(50));
        manager
            .restart_node(
                "offset",
                Box::new(AddOffset {
                    offset: 5000,
                    buffer: VecDeque::new(),
                    scheduled: Arc::new(Mutex::new(VecDeque::new())),
                }),
                vec![sub("source", "out")],
            )
            .unwrap();

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();

        let values = out.lock().unwrap();
        assert!(!values.is_empty());
        assert!(
            values.iter().any(|v| *v >= 5000),
            "restarted node never produced: {values:?}"
        );
        assert!(
            values.iter().any(|v| *v < 5000),
            "old node never produced before restart"
        );
        // Downstream sink survived the restart: it kept collecting after.
        assert!(values.last().copied().unwrap() >= 5000);
    }

    #[test]
    fn stop_all_unblocks_and_joins_everything() {
        let mut manager = manager();
        let out = Arc::new(Mutex::new(Vec::new()));
        manager
            .add_node(NodeSpec {
                name: "source".into(),
                node: Box::new(PacedSource {
                    next: 0,
                    max: i64::MAX, // endless
                    pace: Duration::from_millis(1),
                }),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node(collect_spec("sink", "source", "out", &out))
            .unwrap();

        std::thread::sleep(Duration::from_millis(30));
        let start = std::time::Instant::now();
        manager.stop_all();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "stop_all took too long"
        );
        assert!(!out.lock().unwrap().is_empty());
    }

    /// The interactive stop: `request_stop` must return without joining
    /// (the UI thread calls it mid-frame), and the run must then wind down
    /// on its own so a later `stop_all` reap is instant.
    #[test]
    fn request_stop_is_nonblocking_and_winds_down() {
        let mut manager = manager();
        let out = Arc::new(Mutex::new(Vec::new()));
        manager
            .add_node(NodeSpec {
                name: "source".into(),
                node: Box::new(PacedSource {
                    next: 0,
                    max: i64::MAX, // endless
                    pace: Duration::from_millis(1),
                }),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node(collect_spec("sink", "source", "out", &out))
            .unwrap();

        std::thread::sleep(Duration::from_millis(30));
        let start = std::time::Instant::now();
        manager.request_stop();
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "request_stop must not join node threads"
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !manager.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "run did not wind down after request_stop"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let reap_start = std::time::Instant::now();
        manager.stop_all();
        assert!(
            reap_start.elapsed() < Duration::from_millis(100),
            "reaping a finished run must be instant"
        );
        assert!(!out.lock().unwrap().is_empty());
    }

    #[test]
    fn request_stop_cancels_a_node_blocked_inside_work() {
        let mut manager = manager();
        let (sender, receiver) = crossbeam_channel::bounded(1);
        manager
            .add_node(NodeSpec {
                name: "blocked".into(),
                node: Box::new(BlockingCancelableNode {
                    receiver,
                    cancellation: Arc::new(BlockingCancellation { sender }),
                }),
                inputs: vec![],
            })
            .unwrap();

        manager.request_stop();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !manager.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "cancellation handle did not interrupt blocked work"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        manager.stop_all();
    }

    // ── Capability negotiation ─────────────────────────────────────────

    trait TestQuery: Send + Sync {}

    struct ConstQuery;

    impl TestQuery for ConstQuery {}

    fn test_query_protocol() -> ProtocolKind {
        ProtocolKind::capability::<dyn TestQuery>()
    }

    /// Self-threading source that never streams anything — a well-behaved
    /// consumer of this port has no choice but to use EdgeQuery.
    struct QueryableSource;

    impl ProcessNode for QueryableSource {
        fn name(&self) -> &str {
            "queryable_source"
        }
        fn is_self_threading(&self) -> bool {
            true
        }
        fn should_stop(&self) -> bool {
            true
        }
        fn num_inputs(&self) -> usize {
            0
        }
        fn num_outputs(&self) -> usize {
            1
        }
        fn output_schema(&self) -> Vec<PortSchema> {
            vec![
                PortSchema::state::<TestLevel>("out", 0, PortDirection::Output)
                    .with_protocols(vec![test_query_protocol(), ProtocolKind::Stream]),
            ]
        }
        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            Ok(0)
        }
        fn protocol_capability(
            &self,
            _port: usize,
            protocol: ProtocolKind,
            _input_capabilities: &[Vec<ProtocolCapability>],
        ) -> Option<ProtocolCapability> {
            (protocol == test_query_protocol())
                .then(|| ProtocolCapability::new(Arc::new(ConstQuery) as Arc<dyn TestQuery>))
        }
    }

    /// Passes a producer's random-access capability through while retaining
    /// ordinary stream transport for its runtime input.
    struct DerivedQueryableNode;

    impl ProcessNode for DerivedQueryableNode {
        fn name(&self) -> &str {
            "derived_queryable"
        }
        fn num_inputs(&self) -> usize {
            1
        }
        fn num_outputs(&self) -> usize {
            1
        }
        fn input_schema(&self) -> Vec<PortSchema> {
            vec![PortSchema::state::<TestLevel>(
                "in",
                0,
                PortDirection::Input,
            )]
        }
        fn output_schema(&self) -> Vec<PortSchema> {
            vec![
                PortSchema::state::<TestLevel>("out", 0, PortDirection::Output)
                    .with_protocols(vec![test_query_protocol(), ProtocolKind::Stream]),
            ]
        }
        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            Err(WorkError::Shutdown)
        }
        fn protocol_capability(
            &self,
            port: usize,
            protocol: ProtocolKind,
            input_capabilities: &[Vec<ProtocolCapability>],
        ) -> Option<ProtocolCapability> {
            if port == 0 {
                input_capabilities
                    .first()?
                    .iter()
                    .find(|capability| capability.protocol() == protocol)
                    .cloned()
            } else {
                None
            }
        }
    }

    /// Records whether its input negotiated an EdgeQuery handle, then exits.
    struct QueryProbe {
        got_edge_query: Arc<AtomicBool>,
    }

    impl ProcessNode for QueryProbe {
        fn name(&self) -> &str {
            "query_probe"
        }
        fn num_inputs(&self) -> usize {
            1
        }
        fn num_outputs(&self) -> usize {
            0
        }
        fn input_schema(&self) -> Vec<PortSchema> {
            vec![
                PortSchema::state::<TestLevel>("in", 0, PortDirection::Input)
                    .with_protocols(vec![test_query_protocol(), ProtocolKind::Stream]),
            ]
        }
        fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            if inputs[0].protocol_capability::<dyn TestQuery>().is_some() {
                self.got_edge_query.store(true, Ordering::Relaxed);
            }
            Err(WorkError::Shutdown)
        }
    }

    /// A consumer that deliberately excludes `EdgeQuery` and therefore
    /// requires the streamed protocol.
    struct StreamProbe {
        got_stream: Arc<AtomicBool>,
        buffer: VecDeque<TestLevel>,
    }

    struct CoordinatedStreamProbe {
        got_two_streams: Arc<AtomicBool>,
        buffers: [VecDeque<TestLevel>; 2],
    }

    impl ProcessNode for CoordinatedStreamProbe {
        fn name(&self) -> &str {
            "coordinated_stream_probe"
        }
        fn num_inputs(&self) -> usize {
            2
        }
        fn num_outputs(&self) -> usize {
            0
        }
        fn input_schema(&self) -> Vec<PortSchema> {
            (0..2)
                .map(|index| {
                    PortSchema::state::<TestLevel>(
                        format!("in{index}"),
                        index,
                        PortDirection::Input,
                    )
                    .with_protocols(vec![test_query_protocol(), ProtocolKind::Stream])
                })
                .collect()
        }
        fn select_input_protocols(
            &self,
            candidates: &[Option<InputProtocolCandidate>],
        ) -> Vec<Option<ProtocolKind>> {
            candidates
                .iter()
                .map(|candidate| candidate.as_ref().map(|_| ProtocolKind::Stream))
                .collect()
        }
        fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            let streams = inputs.iter().enumerate().all(|(index, input)| {
                input.protocol_capability::<dyn TestQuery>().is_none()
                    && input.get::<TestLevel>(&mut self.buffers[index]).is_some()
            });
            self.got_two_streams.store(streams, Ordering::Relaxed);
            Err(WorkError::Shutdown)
        }
    }

    impl ProcessNode for StreamProbe {
        fn name(&self) -> &str {
            "stream_probe"
        }
        fn num_inputs(&self) -> usize {
            1
        }
        fn num_outputs(&self) -> usize {
            0
        }
        fn input_schema(&self) -> Vec<PortSchema> {
            vec![
                PortSchema::state::<TestLevel>("in", 0, PortDirection::Input)
                    .with_protocols(vec![ProtocolKind::Stream]),
            ]
        }
        fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            if inputs[0].protocol_capability::<dyn TestQuery>().is_none()
                && inputs[0].get::<TestLevel>(&mut self.buffer).is_some()
            {
                self.got_stream.store(true, Ordering::Relaxed);
            }
            Err(WorkError::Shutdown)
        }
    }

    #[test]
    fn edge_query_negotiated_even_after_producer_already_running() {
        let mut manager = manager();
        manager
            .add_node(NodeSpec {
                name: "source".into(),
                node: Box::new(QueryableSource),
                inputs: vec![],
            })
            .unwrap();

        // Give the (self-threading, should_stop-immediately) source a
        // chance to fully exit — its `Box<dyn ProcessNode>` is gone by
        // then, so this proves the EdgeQuery handle came from the cache in
        // `OutputList`, not a live call into the still-running node.
        std::thread::sleep(Duration::from_millis(50));

        let got = Arc::new(AtomicBool::new(false));
        manager
            .add_node(NodeSpec {
                name: "probe".into(),
                node: Box::new(QueryProbe {
                    got_edge_query: Arc::clone(&got),
                }),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();

        assert!(
            got.load(Ordering::Relaxed),
            "consumer never received an EdgeQuery handle even though both \
             sides declared support for it"
        );
    }

    #[test]
    fn derived_output_receives_streamed_input_query_capability() {
        let mut manager = manager();
        manager
            .add_node_deferred(NodeSpec {
                name: "source".into(),
                node: Box::new(QueryableSource),
                inputs: vec![],
            })
            .unwrap();
        manager
            .add_node_deferred(NodeSpec {
                name: "derived".into(),
                node: Box::new(DerivedQueryableNode),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();

        let got = Arc::new(AtomicBool::new(false));
        manager
            .add_node_deferred(NodeSpec {
                name: "probe".into(),
                node: Box::new(QueryProbe {
                    got_edge_query: Arc::clone(&got),
                }),
                inputs: vec![sub("derived", "out")],
            })
            .unwrap();
        manager.start_all_deferred().unwrap();

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();
        assert!(got.load(Ordering::Relaxed));
    }

    #[test]
    fn stream_only_consumer_overrides_queryable_producer_in_live_manager() {
        let mut manager = manager();
        manager
            .add_node(NodeSpec {
                name: "source".into(),
                node: Box::new(QueryableSource),
                inputs: vec![],
            })
            .unwrap();

        let got = Arc::new(AtomicBool::new(false));
        manager
            .add_node(NodeSpec {
                name: "probe".into(),
                node: Box::new(StreamProbe {
                    got_stream: Arc::clone(&got),
                    buffer: VecDeque::new(),
                }),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();
        assert!(got.load(Ordering::Relaxed));
    }

    #[test]
    fn live_manager_honors_one_coordinated_choice_for_multiple_inputs() {
        let mut manager = manager();
        manager
            .add_node_deferred(NodeSpec {
                name: "source".into(),
                node: Box::new(QueryableSource),
                inputs: vec![],
            })
            .unwrap();

        let got = Arc::new(AtomicBool::new(false));
        manager
            .add_node_deferred(NodeSpec {
                name: "probe".into(),
                node: Box::new(CoordinatedStreamProbe {
                    got_two_streams: Arc::clone(&got),
                    buffers: std::array::from_fn(|_| VecDeque::new()),
                }),
                inputs: vec![sub("source", "out"), sub("source", "out")],
            })
            .unwrap();
        manager.start_all_deferred().unwrap();

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();
        assert!(got.load(Ordering::Relaxed));
    }

    // ── Payload negotiation (live path) ─────────────────────────────────

    /// One output port that can serve `TestValue` and `TestBlock`
    /// destinations simultaneously — sends exactly one of each, then
    /// signals completion. Mirrors `pipeline.rs`'s offline
    /// `MultiKindSource` test, but exercised through the live
    /// `PipelineManager` path this time (the class of gap that let the
    /// `ProtocolKind` work silently miss the live app earlier).
    struct MultiKindSource {
        sent: bool,
    }
    impl ProcessNode for MultiKindSource {
        fn name(&self) -> &str {
            "multi_kind_source"
        }
        fn num_inputs(&self) -> usize {
            0
        }
        fn num_outputs(&self) -> usize {
            1
        }
        fn output_schema(&self) -> Vec<PortSchema> {
            vec![
                PortSchema::state::<TestValue>("out", 0, PortDirection::Output).with_payloads(
                    vec![
                        PortPayload::new::<TestBlock>().with_default_buffer_capacity(2),
                        PortPayload::new::<TestValue>().state(),
                    ],
                ),
            ]
        }
        fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
            if self.sent {
                return Err(WorkError::Shutdown);
            }
            self.sent = true;
            if let Some(sender) = outputs[0].get::<TestValue>() {
                let _ = sender.send(TestValue::new(true, 0));
            }
            if let Some(sender) = outputs[0].get::<TestBlock>() {
                let _ = sender.send(TestBlock::new(Arc::from([0u8].as_slice()), 0, 1, 1));
            }
            Ok(1)
        }
    }

    struct SampleSink {
        got: Arc<Mutex<Vec<TestValue>>>,
    }
    impl ProcessNode for SampleSink {
        fn name(&self) -> &str {
            "sample_sink"
        }
        fn num_inputs(&self) -> usize {
            1
        }
        fn num_outputs(&self) -> usize {
            0
        }
        fn input_schema(&self) -> Vec<PortSchema> {
            vec![PortSchema::state::<TestValue>(
                "in",
                0,
                PortDirection::Input,
            )]
        }
        fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            let mut buf = VecDeque::new();
            let mut recv = inputs[0].get::<TestValue>(&mut buf).unwrap();
            let item = recv.recv()?;
            self.got.lock().unwrap().push(item);
            Ok(1)
        }
    }

    struct BlockSink {
        got: Arc<Mutex<Vec<TestBlock>>>,
    }
    impl ProcessNode for BlockSink {
        fn name(&self) -> &str {
            "block_sink"
        }
        fn num_inputs(&self) -> usize {
            1
        }
        fn num_outputs(&self) -> usize {
            0
        }
        fn input_schema(&self) -> Vec<PortSchema> {
            vec![PortSchema::new::<TestBlock>("in", 0, PortDirection::Input)]
        }
        fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            let mut buf = VecDeque::new();
            let mut recv = inputs[0].get::<TestBlock>(&mut buf).unwrap();
            let item = recv.recv()?;
            self.got.lock().unwrap().push(item);
            Ok(1)
        }
    }

    #[test]
    fn mixed_kind_fan_out_from_one_port_reaches_both_destinations_live() {
        // `TestBlock` isn't a sticky/level type (unlike `TestValue`), so a
        // subscriber added after the source already sent and closed would
        // genuinely miss it — no amount of pacing makes that safe. Add
        // every node deferred and start them together instead, so both
        // sinks are subscribed before the source's thread can run at all
        // (the same guarantee `start_all_deferred`'s own doc describes
        // for initial materialization).
        let mut manager = manager();
        manager
            .add_node_deferred(NodeSpec {
                name: "source".into(),
                node: Box::new(MultiKindSource { sent: false }),
                inputs: vec![],
            })
            .unwrap();

        let sample_got = Arc::new(Mutex::new(Vec::new()));
        let block_got = Arc::new(Mutex::new(Vec::new()));
        manager
            .add_node_deferred(NodeSpec {
                name: "sample_sink".into(),
                node: Box::new(SampleSink {
                    got: sample_got.clone(),
                }),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();
        manager
            .add_node_deferred(NodeSpec {
                name: "block_sink".into(),
                node: Box::new(BlockSink {
                    got: block_got.clone(),
                }),
                inputs: vec![sub("source", "out")],
            })
            .unwrap();
        manager.start_all_deferred().unwrap();

        wait_finished(&manager, Duration::from_secs(5));
        manager.wait();

        assert_eq!(sample_got.lock().unwrap().len(), 1);
        assert_eq!(block_got.lock().unwrap().len(), 1);
    }
}
