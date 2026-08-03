//! Node trait for streaming processing.
//!
//! Defines the ProcessNode trait that all streaming nodes must implement.
//! Nodes actively process data when work() is called by the scheduler.

use std::sync::Arc;

use super::edge_query::EdgeQuery;
use super::errors::WorkResult;
use super::ports::{InputPort, OutputPort};
use super::protocol::ProtocolKind;

/// Producer capabilities considered while a consumer selects an input transport.
#[derive(Clone)]
pub struct InputProtocolCandidate {
    /// Protocols offered by the upstream producer in preference order.
    pub offered: Vec<ProtocolKind>,
    /// Optional random-access capability supplied by the producer.
    pub edge_query: Option<Arc<dyn EdgeQuery>>,
}

/// A configuration value delivered to a running node (live reconfiguration,
/// `docs/APP_DESIGN.md`). Deliberately a tiny bespoke type: the runtime crate stays
/// serde-free and nodes match on plain fields.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    /// Unsigned integer configuration value.
    U64(u64),
    /// Signed integer configuration value.
    I64(i64),
    /// Boolean configuration value.
    Bool(bool),
    /// Text configuration value.
    Text(String),
}

/// Named configuration fields for [`ProcessNode::apply_config`]; produced by
/// the app-layer builders that know how UI state maps onto runtime knobs.
pub type NodeConfig = std::collections::HashMap<String, ConfigValue>;

/// An immutable point on a capture analysis timeline at which a scheduled
/// configuration becomes eligible to take effect.
///
/// `sample_index` is recording-relative. `timestamp_ns` uses the timestamps
/// carried by runtime events, so a node can preserve future-only semantics
/// even when older events are already queued downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigurationBoundary {
    /// Capture-relative sample position at which the change becomes eligible.
    pub sample_index: u64,
    /// Shared timeline timestamp at which the change becomes eligible.
    pub timestamp_ns: u64,
}

impl ConfigurationBoundary {
    /// Creates a future-only configuration boundary on the capture timeline.
    ///
    /// # Parameters
    /// - `sample_index`: Recording-relative sample position.
    /// - `timestamp_ns`: Shared event timestamp in nanoseconds.
    pub const fn new(sample_index: u64, timestamp_ns: u64) -> Self {
        Self {
            sample_index,
            timestamp_ns,
        }
    }
}

/// Thread-safe validation and enqueue boundary for future-only node
/// configuration. The manager retains this handle before moving the node
/// into its worker, so scheduling is not delayed when `work()` is blocked
/// waiting for its next input event.
pub trait ConfigurationScheduler: Send + Sync {
    /// Validates and queues configuration to take effect at a future boundary.
    ///
    /// # Parameters
    /// - `config`: Named runtime configuration values.
    /// - `boundary`: Earliest sample and timestamp at which values may apply.
    fn schedule_config(
        &self,
        config: &NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> ConfigOutcome;
}

/// Thread-safe cancellation endpoint retained by a runtime supervisor after
/// the processing node itself has moved into its worker thread.
///
/// Nodes with internal workers use this to interrupt a blocked `work()`
/// call promptly when an interactive run is stopped.
pub trait NodeCancellation: Send + Sync {
    /// Requests that blocked node work returns promptly for shutdown.
    fn request_cancel(&self);
}

/// Outcome of a hot configuration attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigOutcome {
    /// The change is in effect from the next `work()` on.
    Applied,
    /// The node cannot apply this change while running; the supervisor
    /// restarts it in place.
    NeedsRestart,
}

/// Which connected inputs must be ready before a cooperative scheduler may
/// call a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputScheduling {
    /// Every input is consumed together or may be read during the call.
    #[default]
    All,
    /// The node multiplexes inputs and can make progress from any one of
    /// them without waiting for the others.
    Any,
}

/// Scheduling environment selected by the runtime manager for a node instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuntimeExecutionMode {
    /// The node owns an independently scheduled host task and may block inside it.
    #[default]
    Independent,
    /// The node shares the caller's event loop and must retain resumable bounded state.
    Cooperative,
}

/// Result of one scheduler-visible [`ProcessNode::work`] call.
///
/// `produced_items` remains the value used for node progress counters. The
/// separate `made_progress` bit tells a cooperative runner that the call
/// consumed input or advanced internal state even when it intentionally
/// emitted nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkOutcome {
    produced_items: usize,
    made_progress: bool,
}

impl WorkOutcome {
    /// No input was consumed, no state advanced, and no item was produced.
    pub const fn idle() -> Self {
        Self {
            produced_items: 0,
            made_progress: false,
        }
    }

    /// Derives progress from an existing `work()` produced-item count.
    ///
    /// # Parameters
    /// - `produced_items`: Number of outputs emitted by the completed work call.
    pub const fn from_produced(produced_items: usize) -> Self {
        Self {
            produced_items,
            made_progress: produced_items > 0,
        }
    }

    /// Reports scheduler progress independently of the produced-item count.
    pub const fn progressed(produced_items: usize) -> Self {
        Self {
            produced_items,
            made_progress: true,
        }
    }

    /// Number of output items produced for runtime progress counters.
    pub const fn produced_items(self) -> usize {
        self.produced_items
    }

    /// Whether a cooperative scheduler should continue its current pump.
    pub const fn made_progress(self) -> bool {
        self.made_progress
    }
}

/// Processing-runtime node that transforms streaming data.
///
/// Sources have zero inputs, sinks have zero outputs, and processors connect
/// both directions through type-erased runtime ports.
pub trait ProcessNode: Send {
    /// Returns the runtime debug name for this node instance.
    fn name(&self) -> &str;

    /// Returns whether the node has finished or requested scheduler shutdown.
    fn should_stop(&self) -> bool {
        false
    }

    /// Returns true if this node spawns its own worker threads and manages them internally.
    /// If true, the scheduler will call work() once to start the node, then wait for should_stop().
    /// If false (default), the scheduler will call work() repeatedly in a loop.
    fn is_self_threading(&self) -> bool {
        false
    }

    /// Supplies the execution environment before the runtime inspects or starts the node.
    fn set_runtime_execution_mode(&mut self, _mode: RuntimeExecutionMode) {}

    /// Declares whether `work()` requires all inputs or can multiplex any
    /// ready input. Threaded runners may block inside `work()` and ignore
    /// this; cooperative runners use it to avoid both blocking and needless
    /// head-of-line stalls.
    fn input_scheduling(&self) -> InputScheduling {
        InputScheduling::All
    }

    /// Number of input ports this node requires
    fn num_inputs(&self) -> usize;

    /// Number of output ports this node provides
    fn num_outputs(&self) -> usize;

    /// Get schema for all input ports (name + type + index)
    /// Default implementation returns empty list for backward compatibility
    fn input_schema(&self) -> Vec<crate::ports::PortSchema> {
        Vec::new()
    }

    /// Get schema for all output ports (name + type + index)
    /// Default implementation returns empty list for backward compatibility
    fn output_schema(&self) -> Vec<crate::ports::PortSchema> {
        Vec::new()
    }

    /// Selects one transport per input after producers have exposed their
    /// actual capabilities and optional query metadata. The default keeps
    /// producer preference order. Stateful consumers may override this to
    /// make one coordinated choice across a group of inputs.
    ///
    /// # Parameters
    /// - `candidates`: Upstream protocol and query capabilities by input index.
    fn select_input_protocols(
        &self,
        candidates: &[Option<InputProtocolCandidate>],
    ) -> Vec<Option<ProtocolKind>> {
        let schemas = self.input_schema();
        candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                let candidate = candidate.as_ref()?;
                let accepted = &schemas.get(index)?.protocols;
                candidate
                    .offered
                    .iter()
                    .find(|protocol| accepted.contains(protocol))
                    .copied()
            })
            .collect()
    }

    /// Returns the stable concrete node type identifier for serialization.
    /// Defaults to [`Self::name`].
    fn node_type(&self) -> &str {
        self.name()
    }

    /// Do work: read from inputs, process, write to outputs
    /// The scheduler provides references to input and output port slices
    /// Returns Ok(n) where n is the number of items produced, or Err on failure
    ///
    /// **Cooperative-backend invariant:** implementations must not send more
    /// than one item per output per `work()` call. `CooperativeManager`
    /// (used on wasm) only checks *before* calling `work()` that no output
    /// would currently block (`cooperative_manager`'s module doc);
    /// a node that fans out several sends to the same output within one
    /// call can still fill that output's channel mid-call and hit a real
    /// blocking `send()` — which, on that single-threaded scheduler,
    /// deadlocks the whole pump loop permanently. `PipelineManager`
    /// (thread-per-node, native) has no such constraint — blocking there
    /// only stalls the one node's own thread.
    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize>;

    /// Scheduler-facing form of [`Self::work`]. Nodes that can consume input
    /// or advance state while producing zero items override this method and
    /// return [`WorkOutcome::progressed`].
    fn work_outcome(
        &mut self,
        inputs: &[InputPort],
        outputs: &[OutputPort],
    ) -> WorkResult<WorkOutcome> {
        self.work(inputs, outputs).map(WorkOutcome::from_produced)
    }

    /// Apply a configuration change while running (between `work()` calls).
    /// The default declines, telling the supervisor to restart the node
    /// in place with a freshly built instance.
    fn apply_config(&mut self, _config: &NodeConfig) -> ConfigOutcome {
        ConfigOutcome::NeedsRestart
    }

    /// Returns a thread-safe configuration scheduler when this node supports
    /// explicit future-only epochs. The node and scheduler share a pending
    /// queue; `work()` consumes due entries immediately before processing an
    /// event at or after their timestamp.
    fn configuration_scheduler(&self) -> Option<Arc<dyn ConfigurationScheduler>> {
        None
    }

    /// Returns a thread-safe cancellation endpoint for work that can remain
    /// blocked inside this node after the supervisor requests shutdown.
    fn cancellation(&self) -> Option<Arc<dyn NodeCancellation>> {
        None
    }

    /// Random-access query handle for output port `port`, if this node
    /// can answer it without streaming. Only called by `Pipeline::build`
    /// for connections that negotiated
    /// [`ProtocolKind::EdgeQuery`](super::protocol::ProtocolKind::EdgeQuery)
    /// (see [`PortSchema::protocols`](super::ports::PortSchema::protocols)).
    /// `input_queries` carries this node's own inputs' negotiated query
    /// handles (in `input_schema()` order, `None` where a given input
    /// didn't negotiate `EdgeQuery`) — empty today since only zero-input
    /// source nodes implement this, but a future pass-through node
    /// (e.g. a logic gate) would compose its output's answer from these.
    /// Default: unsupported.
    ///
    /// # Parameters
    /// - `_port`: Output port index for which a query is requested.
    /// - `_input_queries`: Negotiated query capabilities of this node's inputs.
    fn edge_query(
        &self,
        _port: usize,
        _input_queries: &[Option<Arc<dyn EdgeQuery>>],
    ) -> Option<Arc<dyn EdgeQuery>> {
        None
    }
}

/// Forwarding impl so factories (e.g. the graph compiler) can hand
/// `Box<dyn ProcessNode>` to `Pipeline::add_process`.
impl ProcessNode for Box<dyn ProcessNode> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn should_stop(&self) -> bool {
        (**self).should_stop()
    }
    fn is_self_threading(&self) -> bool {
        (**self).is_self_threading()
    }
    fn set_runtime_execution_mode(&mut self, mode: RuntimeExecutionMode) {
        (**self).set_runtime_execution_mode(mode);
    }
    fn input_scheduling(&self) -> InputScheduling {
        (**self).input_scheduling()
    }
    fn num_inputs(&self) -> usize {
        (**self).num_inputs()
    }
    fn num_outputs(&self) -> usize {
        (**self).num_outputs()
    }
    fn input_schema(&self) -> Vec<crate::ports::PortSchema> {
        (**self).input_schema()
    }
    fn output_schema(&self) -> Vec<crate::ports::PortSchema> {
        (**self).output_schema()
    }
    fn cancellation(&self) -> Option<Arc<dyn NodeCancellation>> {
        (**self).cancellation()
    }
    fn select_input_protocols(
        &self,
        candidates: &[Option<InputProtocolCandidate>],
    ) -> Vec<Option<ProtocolKind>> {
        (**self).select_input_protocols(candidates)
    }
    fn node_type(&self) -> &str {
        (**self).node_type()
    }
    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        (**self).work(inputs, outputs)
    }
    fn work_outcome(
        &mut self,
        inputs: &[InputPort],
        outputs: &[OutputPort],
    ) -> WorkResult<WorkOutcome> {
        (**self).work_outcome(inputs, outputs)
    }
    fn apply_config(&mut self, config: &NodeConfig) -> ConfigOutcome {
        (**self).apply_config(config)
    }
    fn configuration_scheduler(&self) -> Option<Arc<dyn ConfigurationScheduler>> {
        (**self).configuration_scheduler()
    }
    fn edge_query(
        &self,
        port: usize,
        input_queries: &[Option<Arc<dyn EdgeQuery>>],
    ) -> Option<Arc<dyn EdgeQuery>> {
        (**self).edge_query(port, input_queries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{PortDirection, PortSchema};

    struct StreamSelectingNode;

    impl ProcessNode for StreamSelectingNode {
        fn name(&self) -> &str {
            "stream_selector"
        }

        fn num_inputs(&self) -> usize {
            1
        }

        fn num_outputs(&self) -> usize {
            0
        }

        fn input_schema(&self) -> Vec<PortSchema> {
            vec![
                PortSchema::new::<u8>("input", 0, PortDirection::Input)
                    .with_protocols(vec![ProtocolKind::EdgeQuery, ProtocolKind::Stream]),
            ]
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

        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            unreachable!()
        }

        fn work_outcome(
            &mut self,
            _inputs: &[InputPort],
            _outputs: &[OutputPort],
        ) -> WorkResult<WorkOutcome> {
            Ok(WorkOutcome::progressed(0))
        }
    }

    #[test]
    fn boxed_process_node_forwards_overridden_contracts() {
        let mut node: Box<dyn ProcessNode> = Box::new(StreamSelectingNode);
        let selected = node.select_input_protocols(&[Some(InputProtocolCandidate {
            offered: vec![ProtocolKind::EdgeQuery, ProtocolKind::Stream],
            edge_query: None,
        })]);
        let outcome = node.work_outcome(&[], &[]).unwrap();

        assert_eq!(selected, vec![Some(ProtocolKind::Stream)]);
        assert_eq!(outcome, WorkOutcome::progressed(0));
    }
}
