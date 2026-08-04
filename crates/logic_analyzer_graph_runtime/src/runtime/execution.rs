use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use logic_analyzer_graph_capabilities::node_support::{
    NodeBuildContext, PortKind, TimelineMarkerReference,
};
use logic_analyzer_graph_plan::{
    CollectedOutputLane, CollectedOutputSubscription, CollectedTableSubscription, ProcessingGraph,
    ProcessingGraphError, ProcessingNode, SamplingOverlayCandidate,
};
use node_graph::api::NodeId;
use signal_processing::{
    AppManager, ArtifactRepository, ConfigurationBoundary, DerivedDataRetention, DerivedLanes,
    DisconnectEvent, InlineWorkExecutor, InputSub, MemoryArtifactRepository, NodeConfig,
    NodeFailure, OverflowPolicy, PersistentStoreConfig, ProcessNode, SampleBlock,
    SamplingPointStore, WorkExecutor,
};

use super::data_collector::DataCollectorBuilder;
use super::{
    ApplyError, RunData, RunDiagnosticRegistry, SourceArtifactReadiness, SourceDataKind,
    SourceReadiness, SourceReadinessRegistry, cache_policy,
};

pub struct GraphRunContext {
    derived_lanes: DerivedLanes,
    /// Storage policy selected by the graph's source. Finite sources retain
    /// their complete timeline; continuous sources can explicitly choose a
    /// bounded rolling window.
    derived_data_retention: DerivedDataRetention,
    derived_word_caches: Vec<Option<PersistentStoreConfig>>,
    timeline_markers: HashMap<TimelineMarkerReference, signal_processing::TimelineMarker>,
    /// Clocked-node sampling overlays resolved during lowering. The host
    /// application independently chooses which candidates to display.
    sampling_overlays: Vec<SamplingOverlayCandidate>,
    sampling_points: HashMap<String, SamplingPointStore>,
    collected_output_subscriptions: Vec<CollectedOutputSubscription>,
    collected_table_subscriptions: Vec<CollectedTableSubscription>,
    diagnostics: RunDiagnosticRegistry,
    source_readiness: SourceReadinessRegistry,
    work_executor: Arc<dyn WorkExecutor>,
    artifact_repository: Arc<dyn ArtifactRepository>,
}

impl Default for GraphRunContext {
    fn default() -> Self {
        Self {
            derived_lanes: DerivedLanes::default(),
            derived_data_retention: DerivedDataRetention::default(),
            derived_word_caches: Vec::new(),
            timeline_markers: HashMap::new(),
            sampling_overlays: Vec::new(),
            sampling_points: HashMap::new(),
            collected_output_subscriptions: Vec::new(),
            collected_table_subscriptions: Vec::new(),
            diagnostics: RunDiagnosticRegistry::default(),
            source_readiness: SourceReadinessRegistry::default(),
            work_executor: Arc::new(InlineWorkExecutor),
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
        }
    }
}

impl GraphRunContext {
    /// Supplies the host-selected bounded work executor to node builders.
    ///
    /// # Parameters
    /// - `executor`: Input consumed by this operation.
    pub fn set_work_executor(&mut self, executor: Arc<dyn WorkExecutor>) {
        self.work_executor = executor;
    }

    /// Supplies the host-selected repository used by concrete data stores.
    pub fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>) {
        self.artifact_repository = repository;
    }

    /// Returns the run's collected lanes for binding to host views and panels.
    pub fn derived_lanes(&self) -> &DerivedLanes {
        &self.derived_lanes
    }

    /// Takes the sampling-overlay candidates discovered for this run.
    pub fn take_sampling_overlays(&mut self) -> Vec<SamplingOverlayCandidate> {
        std::mem::take(&mut self.sampling_overlays)
    }

    /// Supplies a shared lane catalog that a deferred execution host can populate later.
    pub fn set_derived_lanes(&mut self, lanes: DerivedLanes) {
        self.derived_lanes = lanes;
    }

    /// Supplies statically lowered overlay metadata before deferred execution starts.
    pub fn set_sampling_overlays(&mut self, overlays: Vec<SamplingOverlayCandidate>) {
        self.sampling_overlays = overlays;
    }

    /// Returns application-requested retained outputs and their resolved lane metadata.
    pub fn collected_output_subscriptions(&self) -> &[CollectedOutputSubscription] {
        &self.collected_output_subscriptions
    }

    /// Returns requested retained-table subscriptions resolved for this run.
    pub fn collected_table_subscriptions(&self) -> &[CollectedTableSubscription] {
        &self.collected_table_subscriptions
    }

    /// Returns all application-neutral data and readiness handles for this run.
    pub fn run_data(&self) -> RunData {
        RunData::new(
            self.derived_lanes.clone(),
            self.collected_output_subscriptions.clone(),
            self.collected_table_subscriptions.clone(),
            self.sampling_overlays.clone(),
            self.diagnostics.clone(),
            self.source_readiness.clone(),
        )
    }

    /// Returns runtime diagnostics registered during lowering and materialization.
    pub fn diagnostics(&self) -> &RunDiagnosticRegistry {
        &self.diagnostics
    }

    /// Returns readiness handles for discovered capture sources.
    pub fn source_readiness(&self) -> &SourceReadinessRegistry {
        &self.source_readiness
    }

    /// Supplies one host-owned timeline position to nodes materialized for
    /// this run. Values are snapshots; changing the host position takes
    /// effect on the next run.
    pub fn set_timeline_marker(
        &mut self,
        reference: TimelineMarkerReference,
        marker: signal_processing::TimelineMarker,
    ) {
        self.timeline_markers.insert(reference, marker);
    }

    pub fn timeline_markers(
        &self,
    ) -> impl Iterator<Item = (&TimelineMarkerReference, &signal_processing::TimelineMarker)> {
        self.timeline_markers.iter()
    }
}

impl NodeBuildContext for GraphRunContext {
    fn derived_lanes(&self) -> &DerivedLanes {
        &self.derived_lanes
    }

    fn derived_data_retention(&self) -> DerivedDataRetention {
        self.derived_data_retention
    }

    fn derived_word_cache(&self, member: usize) -> Option<&PersistentStoreConfig> {
        self.derived_word_caches
            .get(member)
            .and_then(Option::as_ref)
    }

    fn sampling_points(&self, runtime_name: &str) -> Option<SamplingPointStore> {
        self.sampling_points.get(runtime_name).cloned()
    }

    fn work_executor(&self) -> Arc<dyn WorkExecutor> {
        Arc::clone(&self.work_executor)
    }

    fn artifact_repository(&self) -> Arc<dyn ArtifactRepository> {
        Arc::clone(&self.artifact_repository)
    }

    fn timeline_marker(
        &self,
        reference: TimelineMarkerReference,
    ) -> Option<signal_processing::TimelineMarker> {
        self.timeline_markers.get(&reference).copied()
    }
}

fn publish_materialized_source_readiness(
    compiled: &ProcessingGraph,
    readiness: &SourceReadinessRegistry,
) {
    for node in &compiled.nodes {
        let Some(lifecycle) = node.materializer.source_data_lifecycle() else {
            continue;
        };
        readiness.publish(SourceReadiness {
            source: node.id,
            kind: match lifecycle.kind {
                logic_analyzer_graph_capabilities::node_support::SourceDataLifecycleKind::File => {
                    SourceDataKind::File
                }
                logic_analyzer_graph_capabilities::node_support::SourceDataLifecycleKind::Live => {
                    SourceDataKind::Live
                }
            },
            preload: if lifecycle.preload {
                SourceArtifactReadiness::Pending
            } else {
                SourceArtifactReadiness::Unsupported
            },
            cache: if lifecycle.cache {
                SourceArtifactReadiness::Pending
            } else {
                SourceArtifactReadiness::Unsupported
            },
            index: if lifecycle.index {
                SourceArtifactReadiness::Pending
            } else {
                SourceArtifactReadiness::Unsupported
            },
            data: match lifecycle.kind {
                logic_analyzer_graph_capabilities::node_support::SourceDataLifecycleKind::File => {
                    SourceArtifactReadiness::Pending
                }
                logic_analyzer_graph_capabilities::node_support::SourceDataLifecycleKind::Live => {
                    SourceArtifactReadiness::Available
                }
            },
        });
    }
}

/// Discovers a concrete source's pre-run presentation through its builder contract.
fn sampling_point_map(compiled: &ProcessingGraph) -> HashMap<String, SamplingPointStore> {
    compiled
        .sampling_overlays
        .iter()
        .map(|candidate| {
            (
                compiled_node(compiled, candidate.node_id())
                    .runtime_name
                    .clone(),
                candidate.overlay().points.clone(),
            )
        })
        .collect()
}

fn reuse_sampling_points(previous: &ProcessingGraph, next: &mut ProcessingGraph) {
    for candidate in &mut next.sampling_overlays {
        let Some(previous_candidate) = previous
            .sampling_overlays
            .iter()
            .find(|previous| previous.node_id() == candidate.node_id())
        else {
            continue;
        };
        candidate.set_points(previous_candidate.overlay().points.clone());
    }
}

/// Producers-before-consumers order; `lower` already rejected cycles.
fn topo_order(compiled: &ProcessingGraph) -> Vec<NodeId> {
    let mut indegree: HashMap<NodeId, usize> =
        compiled.nodes.iter().map(|node| (node.id, 0)).collect();
    for edge in &compiled.edges {
        *indegree.entry(edge.to.0).or_default() += 1;
    }
    let mut queue: Vec<NodeId> = compiled
        .nodes
        .iter()
        .map(|node| node.id)
        .filter(|id| indegree[id] == 0)
        .collect();
    queue.sort_by_key(|id| id.0);
    let mut order = Vec::with_capacity(compiled.nodes.len());
    while let Some(id) = queue.pop() {
        order.push(id);
        for edge in compiled.edges.iter().filter(|edge| edge.from.0 == id) {
            let degree = indegree.get_mut(&edge.to.0).expect("kept node");
            *degree -= 1;
            if *degree == 0 {
                queue.push(edge.to.0);
            }
        }
    }
    order
}

pub(crate) fn compiled_node(compiled: &ProcessingGraph, id: NodeId) -> &ProcessingNode {
    compiled
        .nodes
        .iter()
        .find(|node| node.id == id)
        .expect("node in compiled graph")
}

fn materialize_compiled_node(
    node: &ProcessingNode,
    runtime_name: &str,
    graph: &ProcessingGraph,
    ctx: &mut GraphRunContext,
) -> Result<Box<dyn ProcessNode>, String> {
    let builder = node.materializer.as_ref();
    if builder.is_data_subscription() || builder.is_data_collector() {
        return DataCollectorBuilder::build_with_lane_names(
            runtime_name,
            &node.resolved,
            &builder.collected_lane_names(&node.state, &node.resolved),
            graph.payload_catalog.as_ref(),
            ctx,
        );
    }
    builder.build(runtime_name, &node.state, &node.resolved, ctx)
}

fn collected_table_subscriptions(compiled: &ProcessingGraph) -> Vec<CollectedTableSubscription> {
    compiled
        .nodes
        .iter()
        .filter(|node| node.data_collector)
        .filter_map(|node| {
            let builder = node.materializer.as_ref();
            let lanes = builder
                .collected_lane_names(&node.state, &node.resolved)
                .into_iter()
                .filter_map(|(member, lane_name)| {
                    let input = node.resolved.get(0, member)?.clone();
                    input
                        .decoder_table_column
                        .is_some()
                        .then_some(CollectedOutputLane {
                            member,
                            lane_name,
                            source_label: input.source_node_title.clone(),
                            input,
                        })
                })
                .collect::<Vec<_>>();
            (!lanes.is_empty()).then_some(CollectedTableSubscription {
                collector: node.id,
                lanes,
            })
        })
        .collect()
}

fn collected_output_subscriptions(compiled: &ProcessingGraph) -> Vec<CollectedOutputSubscription> {
    compiled
        .nodes
        .iter()
        .filter_map(|node| {
            let builder = node.materializer.as_ref();
            node.data_collector
                .then(|| {
                    let lanes: Vec<CollectedOutputLane> = builder
                        .collected_lane_names(&node.state, &node.resolved)
                        .into_iter()
                        .filter_map(|(member, lane_name)| {
                            node.resolved.get(0, member).cloned().and_then(|input| {
                                compiled
                                    .output_subscriptions
                                    .contains(input.source_node, input.source_output)
                                    .then(|| CollectedOutputLane {
                                        source_label: builder.collected_source_label(
                                            &node.state,
                                            &input.source_node_title,
                                        ),
                                        member,
                                        lane_name,
                                        input,
                                    })
                            })
                        })
                        .collect();
                    (!lanes.is_empty()).then_some(CollectedOutputSubscription {
                        runtime_name: node.runtime_name.clone(),
                        lanes,
                    })
                })
                .flatten()
        })
        .collect()
}

pub(crate) fn derived_cache_configs_by_node_with_subscriptions(
    compiled: &ProcessingGraph,
    repository: &Arc<dyn ArtifactRepository>,
) -> HashMap<NodeId, Vec<PersistentStoreConfig>> {
    cache_policy::cache_configs_by_node(compiled, repository)
}

/// Input subscriptions for `id`, matched to the built node's input schema.
fn input_subs(
    compiled: &ProcessingGraph,
    id: NodeId,
    built: &dyn ProcessNode,
    names: &HashMap<NodeId, String>,
) -> Result<Vec<Option<InputSub>>, String> {
    built
        .input_schema()
        .iter()
        .map(|schema| {
            let edge = compiled
                .edges
                .iter()
                .find(|edge| edge.to.0 == id && edge.to.1 == schema.name);
            match edge {
                None => Ok(None),
                Some(edge) => {
                    let from_node = names
                        .get(&edge.from.0)
                        .ok_or_else(|| format!("producer n{} not materialized", edge.from.0.0))?;
                    Ok(Some(InputSub {
                        from_node: from_node.clone(),
                        from_port: edge.from.1.clone(),
                        buffer: edge.buffer,
                        policy: OverflowPolicy::Block,
                    }))
                }
            }
        })
        .collect()
}

/// One live edit, in application order (removals reverse-topological,
/// additions topological, then hot configs and in-place restarts).
#[derive(Debug)]
enum LiveEdit {
    Remove(NodeId),
    Add(NodeId),
    Configure(NodeId, NodeConfig),
    Restart(NodeId),
}

/// Summary of changes applied to a running graph without a full restart.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ApplySummary {
    /// Nodes materialized into the running graph.
    pub added: usize,
    /// Nodes removed from the running graph.
    pub removed: usize,
    /// Nodes updated with in-place configuration.
    pub configured: usize,
    /// Nodes rebuilt in place after a compatible change.
    pub restarted: usize,
}

impl ApplySummary {
    /// Returns whether no live changes were applied.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Wiring signature of a node's inputs, for diffing.
fn wiring_of(compiled: &ProcessingGraph, id: NodeId) -> BTreeSet<(String, u32, String, usize)> {
    compiled
        .edges
        .iter()
        .filter(|edge| edge.to.0 == id)
        .map(|edge| {
            (
                edge.to.1.clone(),
                edge.from.0.0,
                edge.from.1.clone(),
                edge.buffer,
            )
        })
        .collect()
}

/// Classifies the difference between the running IR and the edited one
/// (the edit classes of `docs/APP_DESIGN.md`). Returns the edit list, or
/// the reason a full restart is needed.
fn diff(old: &ProcessingGraph, new: &ProcessingGraph) -> Result<Vec<LiveEdit>, String> {
    let old_ids: HashSet<NodeId> = old.nodes.iter().map(|node| node.id).collect();
    let new_ids: HashSet<NodeId> = new.nodes.iter().map(|node| node.id).collect();
    let is_source = |compiled: &ProcessingGraph, id: NodeId| {
        compiled_node(compiled, id)
            .materializer
            .is_time_domain_source()
    };

    let mut edits: Vec<LiveEdit> = Vec::new();

    // Removals, consumers before producers.
    let mut removals: Vec<NodeId> = topo_order(old)
        .into_iter()
        .rev()
        .filter(|id| !new_ids.contains(id))
        .collect();
    for &id in &removals {
        if is_source(old, id) {
            return Err("the source node was removed".into());
        }
    }
    edits.extend(removals.drain(..).map(LiveEdit::Remove));

    // Additions, producers before consumers.
    for id in topo_order(new) {
        if old_ids.contains(&id) {
            continue;
        }
        for edge in new.edges.iter().filter(|edge| edge.to.0 == id) {
            if edge.kind == PortKind::of::<SampleBlock>() {
                return Err(
                    "new node consumes block channels; block subscriptions cannot join mid-stream"
                        .to_string(),
                );
            }
            if is_source(new, edge.from.0) {
                return Err(
                    "new connection directly to the source; source destinations are fixed at start"
                        .into(),
                );
            }
        }
        edits.push(LiveEdit::Add(id));
    }

    // Changed nodes: hot config, or restart in place.
    for id in topo_order(new) {
        if !old_ids.contains(&id) {
            continue;
        }
        let old_node = compiled_node(old, id);
        let new_node = compiled_node(new, id);
        let wiring_changed = wiring_of(old, id) != wiring_of(new, id);
        let builder = new_node.materializer.as_ref();
        let state_changed =
            builder.execution_state(&old_node.state) != builder.execution_state(&new_node.state);
        if !wiring_changed && !state_changed {
            continue;
        }
        if is_source(new, id) {
            return Err("the source node changed".into());
        }
        if !wiring_changed
            && state_changed
            && let Some(config) = builder.hot_config(&new_node.state)
        {
            edits.push(LiveEdit::Configure(id, config));
            continue;
        }
        // Restart in place: the node re-subscribes to its producers, which
        // is invisible to block streams and to source ports (their worker
        // threads snapshot destinations at start).
        for edge in new.edges.iter().filter(|edge| edge.to.0 == id) {
            if edge.kind == PortKind::of::<SampleBlock>() {
                return Err(format!(
                    "'{}' consumes block channels and cannot restart mid-stream",
                    new_node.runtime_name
                ));
            }
            if is_source(new, edge.from.0) {
                return Err(format!(
                    "'{}' is fed directly by the source and cannot restart mid-stream",
                    new_node.runtime_name
                ));
            }
        }
        edits.push(LiveEdit::Restart(id));
    }

    Ok(edits)
}

/// A pipeline running under the live supervisor: editable while it runs.
pub struct LiveRun {
    manager: AppManager,
    compiled: ProcessingGraph,
    /// Supervisor key per UI node — assigned at add time and stable across
    /// title renames and in-place restarts.
    names: HashMap<NodeId, String>,
    lanes: DerivedLanes,
    collected_output_subscriptions: Vec<CollectedOutputSubscription>,
    collected_table_subscriptions: Vec<CollectedTableSubscription>,
    diagnostics: RunDiagnosticRegistry,
    source_readiness: SourceReadinessRegistry,
    /// Set by [`Self::stop`]: the wind-down has been signalled but node
    /// threads may still be finishing their current `work()` call.
    stop_requested: bool,
    cache_pruned: bool,
    timeline_markers: HashMap<TimelineMarkerReference, signal_processing::TimelineMarker>,
    work_executor: Arc<dyn WorkExecutor>,
    artifact_repository: Arc<dyn ArtifactRepository>,
}

/// One provider-owned source process used only while a live capture follows
/// its authoritative store.
pub struct LiveAnalysisSource {
    /// Live-capture source node replaced by this provider-owned process.
    pub source_node: NodeId,
    /// Process that follows the provider's authoritative capture store.
    pub process: Box<dyn ProcessNode>,
}

/// Explicit source-node replacements used when materializing a graph.
///
/// The compiler validates every node ID against the lowered graph and never
/// interprets the source process or discovers a provider. Live capture and
/// finalized replay therefore share one substitution mechanism.
pub type SourceProcessOverrides = HashMap<NodeId, Box<dyn ProcessNode>>;

/// Lowers and materializes `graph` under a host-selected [`AppManager`].
/// Publishes valid persistent derived lanes without executing the processing graph.
pub(crate) fn load_cached_data_with_subscriptions(
    mut compiled: ProcessingGraph,
    ctx: &mut GraphRunContext,
) -> Result<bool, Vec<ProcessingGraphError>> {
    cache_policy::assign_derived_word_caches(&mut compiled);
    cache_policy::assign_sampling_point_caches(&mut compiled);
    cache_policy::configure_repository(&mut compiled, &ctx.artifact_repository);
    let sampling_cache_loaded = cache_policy::open_sampling_point_stores(
        &mut compiled,
        &ctx.derived_lanes,
        &ctx.artifact_repository,
    );
    let preview = cache_policy::prepare_cached_preview(&compiled);
    if preview.is_none() && !sampling_cache_loaded {
        cache_policy::schedule_maintenance(&compiled, &ctx.artifact_repository, &ctx.work_executor);
        return Ok(false);
    }

    ctx.derived_data_retention = compiled.derived_data_retention;
    ctx.sampling_overlays
        .clone_from(&compiled.sampling_overlays);
    ctx.sampling_points = sampling_point_map(&compiled);
    ctx.collected_output_subscriptions = preview
        .as_ref()
        .map(collected_output_subscriptions)
        .unwrap_or_default();
    ctx.collected_table_subscriptions = preview
        .as_ref()
        .map(collected_table_subscriptions)
        .unwrap_or_default();

    if let Some(preview) = &preview {
        for node in &preview.nodes {
            ctx.derived_word_caches
                .clone_from(&node.derived_word_caches);
            materialize_compiled_node(node, &node.runtime_name, preview, ctx)
                .map_err(|message| vec![ProcessingGraphError::on(node.id, message)])?;
        }
    }
    cache_policy::schedule_maintenance(&compiled, &ctx.artifact_repository, &ctx.work_executor);
    Ok(true)
}

/// Starts the fixed compiled graph with its live-capable source replaced by
/// the process that follows the capture store. All other nodes use the same
/// lowering and materialization path as an ordinary run.
pub(crate) fn start_live_analysis_with_subscriptions(
    compiled: ProcessingGraph,
    ctx: &mut GraphRunContext,
    source: LiveAnalysisSource,
    runtime_factory: &dyn signal_processing::AppManagerFactory,
) -> Result<LiveRun, Vec<ProcessingGraphError>> {
    let mut overrides = SourceProcessOverrides::new();
    overrides.insert(source.source_node, source.process);
    start_live_inner(compiled, ctx, overrides, runtime_factory)
}

fn start_live_inner(
    mut compiled: ProcessingGraph,
    ctx: &mut GraphRunContext,
    mut source_overrides: SourceProcessOverrides,
    runtime_factory: &dyn signal_processing::AppManagerFactory,
) -> Result<LiveRun, Vec<ProcessingGraphError>> {
    cache_policy::assign_derived_word_caches(&mut compiled);
    cache_policy::assign_sampling_point_caches(&mut compiled);
    cache_policy::configure_repository(&mut compiled, &ctx.artifact_repository);
    let (execution, cache_pruned) = cache_policy::prepare_execution(&compiled);
    cache_policy::prepare_sampling_point_stores(
        &mut compiled,
        &execution,
        &ctx.derived_lanes,
        &ctx.artifact_repository,
        &ctx.work_executor,
    );
    ctx.derived_data_retention = compiled.derived_data_retention;
    ctx.sampling_overlays
        .clone_from(&compiled.sampling_overlays);
    ctx.sampling_points = sampling_point_map(&compiled);
    ctx.collected_output_subscriptions = collected_output_subscriptions(&compiled);
    ctx.collected_table_subscriptions = collected_table_subscriptions(&compiled);
    let mut manager = runtime_factory.create();
    let mut names: HashMap<NodeId, String> = HashMap::new();

    for source_node in source_overrides.keys().copied() {
        let Some(node) = execution.nodes.iter().find(|node| node.id == source_node) else {
            return Err(vec![ProcessingGraphError::on(
                source_node,
                "source override is not retained by the compiled graph",
            )]);
        };
        let is_source = node.materializer.is_time_domain_source();
        if !is_source {
            return Err(vec![ProcessingGraphError::on(
                source_node,
                "source override does not target a source node",
            )]);
        }
    }

    for id in topo_order(&execution) {
        let node = compiled_node(&execution, id);
        ctx.derived_word_caches
            .clone_from(&node.derived_word_caches);
        let process = if let Some(process) = source_overrides.remove(&id) {
            process
        } else {
            materialize_compiled_node(node, &node.runtime_name, &execution, ctx)
                .map_err(|message| vec![ProcessingGraphError::on(id, message)])?
        };
        let inputs = input_subs(&execution, id, process.as_ref(), &names)
            .map_err(|message| vec![ProcessingGraphError::on(id, message)])?;
        manager
            .add_node_deferred(signal_processing::NodeSpec {
                name: node.runtime_name.clone(),
                node: process,
                inputs,
            })
            .map_err(|message| vec![ProcessingGraphError::on(id, message)])?;
        names.insert(id, node.runtime_name.clone());
    }
    // All initial subscriptions exist; only now may threads start (a
    // self-threading source snapshots its subscriber lists on first work()).
    manager
        .start_all_deferred()
        .map_err(|message| vec![ProcessingGraphError::global(message)])?;
    publish_materialized_source_readiness(&compiled, &ctx.source_readiness);

    Ok(LiveRun {
        manager,
        compiled,
        names,
        lanes: ctx.derived_lanes.clone(),
        collected_output_subscriptions: ctx.collected_output_subscriptions.clone(),
        collected_table_subscriptions: ctx.collected_table_subscriptions.clone(),
        diagnostics: ctx.diagnostics.clone(),
        source_readiness: ctx.source_readiness.clone(),
        stop_requested: false,
        cache_pruned,
        timeline_markers: ctx.timeline_markers.clone(),
        work_executor: Arc::clone(&ctx.work_executor),
        artifact_repository: Arc::clone(&ctx.artifact_repository),
    })
}

impl LiveRun {
    /// Returns sampling overlays resolved from the compiled graph.
    pub fn sampling_overlays(&self) -> &[SamplingOverlayCandidate] {
        &self.compiled.sampling_overlays
    }

    /// Returns persistent derived-word cache configurations for this run.
    pub fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig> {
        self.compiled
            .nodes
            .iter()
            .flat_map(|node| node.derived_word_caches.iter().flatten().cloned())
            .collect()
    }

    /// Returns resolved retained-output subscriptions.
    pub fn collected_output_subscriptions(&self) -> &[CollectedOutputSubscription] {
        &self.collected_output_subscriptions
    }

    /// Returns resolved retained-table subscriptions.
    pub fn collected_table_subscriptions(&self) -> &[CollectedTableSubscription] {
        &self.collected_table_subscriptions
    }

    /// Returns the run-owned catalog of collected derived lanes.
    pub fn derived_lanes(&self) -> &DerivedLanes {
        &self.lanes
    }

    /// Returns a coherent application-neutral snapshot of this live run.
    pub fn run_data(&self) -> RunData {
        RunData::new(
            self.lanes.clone(),
            self.collected_output_subscriptions.clone(),
            self.collected_table_subscriptions.clone(),
            self.compiled.sampling_overlays.clone(),
            self.diagnostics.clone(),
            self.source_readiness.clone(),
        )
    }

    /// Returns runtime diagnostics for this live run.
    pub fn diagnostics(&self) -> &RunDiagnosticRegistry {
        &self.diagnostics
    }

    /// Returns readiness handles for this live run's capture sources.
    pub fn source_readiness(&self) -> &SourceReadinessRegistry {
        &self.source_readiness
    }

    /// Diffs the edited graph against what is running and applies the
    /// difference live. On any error the running pipeline is untouched
    /// (edits either fail up front in `diff`, or — for build failures midway
    /// — leave already-applied edits in place and report).
    pub(crate) fn apply_compiled(
        &mut self,
        mut new: ProcessingGraph,
    ) -> Result<ApplySummary, ApplyError> {
        cache_policy::assign_derived_word_caches(&mut new);
        cache_policy::assign_sampling_point_caches(&mut new);
        reuse_sampling_points(&self.compiled, &mut new);
        cache_policy::configure_repository(&mut new, &self.artifact_repository);
        let edits = diff(&self.compiled, &new).map_err(ApplyError::NeedsFullRestart)?;
        if edits.is_empty() {
            self.collected_output_subscriptions = collected_output_subscriptions(&new);
            self.collected_table_subscriptions = collected_table_subscriptions(&new);
            self.compiled = new;
            return Ok(ApplySummary::default());
        }
        if self.cache_pruned {
            return Err(ApplyError::NeedsFullRestart(
                "the running graph reused persistent derived data; stop and rerun to apply edits"
                    .to_string(),
            ));
        }

        let mut ctx = GraphRunContext {
            derived_lanes: self.lanes.clone(),
            derived_data_retention: new.derived_data_retention,
            derived_word_caches: Vec::new(),
            sampling_overlays: new.sampling_overlays.clone(),
            sampling_points: sampling_point_map(&new),
            collected_output_subscriptions: collected_output_subscriptions(&new),
            collected_table_subscriptions: collected_table_subscriptions(&new),
            diagnostics: self.diagnostics.clone(),
            source_readiness: self.source_readiness.clone(),
            timeline_markers: self.timeline_markers.clone(),
            work_executor: Arc::clone(&self.work_executor),
            artifact_repository: Arc::clone(&self.artifact_repository),
        };
        let mut summary = ApplySummary::default();
        for edit in edits {
            match edit {
                LiveEdit::Remove(id) => {
                    if let Some(name) = self.names.remove(&id) {
                        self.manager.remove_node(&name).map_err(ApplyError::Apply)?;
                    }
                    summary.removed += 1;
                }
                LiveEdit::Add(id) => {
                    let node = compiled_node(&new, id);
                    ctx.derived_word_caches
                        .clone_from(&node.derived_word_caches);
                    let process =
                        materialize_compiled_node(node, &node.runtime_name, &new, &mut ctx)
                            .map_err(ApplyError::Apply)?;
                    let inputs = input_subs(&new, id, process.as_ref(), &self.names)
                        .map_err(ApplyError::Apply)?;
                    self.manager
                        .add_node(signal_processing::NodeSpec {
                            name: node.runtime_name.clone(),
                            node: process,
                            inputs,
                        })
                        .map_err(ApplyError::Apply)?;
                    self.names.insert(id, node.runtime_name.clone());
                    summary.added += 1;
                }
                LiveEdit::Configure(id, config) => {
                    let name = self
                        .names
                        .get(&id)
                        .ok_or_else(|| ApplyError::Apply(format!("n{} not running", id.0)))?;
                    self.manager
                        .reconfigure(name, config)
                        .map_err(ApplyError::Apply)?;
                    summary.configured += 1;
                }
                LiveEdit::Restart(id) => {
                    let node = compiled_node(&new, id);
                    let name = self
                        .names
                        .get(&id)
                        .cloned()
                        .ok_or_else(|| ApplyError::Apply(format!("n{} not running", id.0)))?;
                    ctx.derived_word_caches
                        .clone_from(&node.derived_word_caches);
                    let process = materialize_compiled_node(node, &name, &new, &mut ctx)
                        .map_err(ApplyError::Apply)?;
                    let inputs = input_subs(&new, id, process.as_ref(), &self.names)
                        .map_err(ApplyError::Apply)?;
                    self.manager
                        .restart_node(&name, process, inputs)
                        .map_err(ApplyError::Apply)?;
                    summary.restarted += 1;
                }
            }
        }
        self.collected_output_subscriptions = collected_output_subscriptions(&new);
        self.collected_table_subscriptions = collected_table_subscriptions(&new);
        self.compiled = new;
        Ok(summary)
    }

    /// Applies the subset of an edited capture graph that can preserve an
    /// explicit future-only boundary. Phase 13.1 deliberately accepts only
    /// builder-declared hot configuration; structural changes and restarts
    /// remain in the edited graph for the next capture or ordinary Run.
    pub(crate) fn apply_configuration_epoch_compiled(
        &mut self,
        mut new: ProcessingGraph,
        boundary: ConfigurationBoundary,
    ) -> Result<ApplySummary, ApplyError> {
        cache_policy::assign_derived_word_caches(&mut new);
        cache_policy::assign_sampling_point_caches(&mut new);
        reuse_sampling_points(&self.compiled, &mut new);
        cache_policy::configure_repository(&mut new, &self.artifact_repository);
        let edits = diff(&self.compiled, &new).map_err(ApplyError::NeedsFullRestart)?;
        if edits.is_empty() {
            self.collected_output_subscriptions = collected_output_subscriptions(&new);
            self.collected_table_subscriptions = collected_table_subscriptions(&new);
            self.compiled = new;
            return Ok(ApplySummary::default());
        }
        if self.cache_pruned {
            return Err(ApplyError::NeedsFullRestart(
                "the running graph reused persistent derived data; the edit is deferred to the next capture"
                    .to_string(),
            ));
        }
        if let Some(edit) = edits
            .iter()
            .find(|edit| !matches!(edit, LiveEdit::Configure(_, _)))
        {
            let reason = match edit {
                LiveEdit::Add(_) => "node additions",
                LiveEdit::Remove(_) => "node removals",
                LiveEdit::Restart(_) => "node restarts or wiring changes",
                LiveEdit::Configure(_, _) => unreachable!(),
            };
            return Err(ApplyError::NeedsFullRestart(format!(
                "{reason} are deferred to the next capture"
            )));
        }

        // Resolve every target before sending any control message so a
        // missing running node cannot leave a partially scheduled epoch.
        let scheduled: Vec<_> = edits
            .into_iter()
            .map(|edit| match edit {
                LiveEdit::Configure(id, config) => self
                    .names
                    .get(&id)
                    .cloned()
                    .map(|name| (name, config))
                    .ok_or_else(|| ApplyError::Apply(format!("n{} not running", id.0))),
                _ => unreachable!(),
            })
            .collect::<Result<_, _>>()?;
        let configured = scheduled.len();
        for (name, config) in scheduled {
            self.manager
                .reconfigure_at(&name, config, boundary)
                .map_err(ApplyError::Apply)?;
        }
        self.collected_output_subscriptions = collected_output_subscriptions(&new);
        self.collected_table_subscriptions = collected_table_subscriptions(&new);
        self.compiled = new;
        Ok(ApplySummary {
            configured,
            ..ApplySummary::default()
        })
    }

    /// Returns whether finished.
    pub fn is_finished(&self) -> bool {
        self.manager.is_finished()
    }

    /// Signals the wind-down and returns immediately — never joins node
    /// threads, so it is safe to call from the frame loop (a node may be
    /// mid-`work()` for a while yet; see `PipelineManager::request_stop`).
    /// [`Self::is_finished`] flips once every thread has exited.
    pub fn stop(&mut self) {
        self.stop_requested = true;
        self.manager.request_stop();
    }

    /// True from [`Self::stop`] until the run is dropped — used by the
    /// toolbar to show "Stopping…" while threads finish their current
    /// `work()` call.
    pub fn is_stopping(&self) -> bool {
        self.stop_requested
    }

    /// Drives up to `budget` `work()` calls forward. A no-op on the
    /// threaded native manager (its nodes run themselves); on wasm's
    /// cooperative manager this is what actually advances the run, so the
    /// UI frame loop must call it every frame regardless of target.
    ///
    /// # Parameters
    /// - `budget`: Input consumed by this operation.
    pub fn pump(&mut self, budget: usize) {
        self.manager.pump(budget);
    }

    /// Drives cooperative work without monopolizing an interactive host event loop.
    pub fn pump_for(&mut self, budget: usize, max_duration: std::time::Duration) {
        self.manager.pump_for(budget, max_duration);
    }

    /// Blocks until the run completes naturally (tests / headless).
    pub fn wait(&mut self) {
        self.manager.wait();
    }

    /// Items produced per UI node (sum of `work()` returns), for header
    /// progress display.
    pub fn progress(&self) -> Vec<(NodeId, u64)> {
        let by_name: HashMap<String, u64> = self.manager.progress().into_iter().collect();
        self.names
            .iter()
            .filter_map(|(id, name)| by_name.get(name).map(|items| (*id, *items)))
            .collect()
    }

    /// Consumers dropped by backpressure policy since the last call, mapped
    /// back to UI nodes where possible.
    pub fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)> {
        self.manager
            .take_disconnected()
            .into_iter()
            .map(|event| {
                let id = event.consumer.as_ref().and_then(|consumer| {
                    self.names
                        .iter()
                        .find(|(_, name)| *name == consumer)
                        .map(|(id, _)| *id)
                });
                (id, event)
            })
            .collect()
    }

    /// Terminal node failures since the last call, mapped back to UI nodes.
    pub fn take_node_failures(&mut self) -> Vec<(Option<NodeId>, NodeFailure)> {
        self.manager
            .take_failures()
            .into_iter()
            .map(|failure| {
                let id = self
                    .names
                    .iter()
                    .find(|(_, name)| **name == failure.node)
                    .map(|(id, _)| *id);
                (id, failure)
            })
            .collect()
    }
}

/// Starts an ordinary application run while replacing explicitly identified
/// source nodes. Finalized-session replay uses this entry point so lowering
/// cannot invoke the captured provider's discovery or build paths.
pub(crate) fn start_app_run_with_source_overrides_and_subscriptions(
    compiled: ProcessingGraph,
    ctx: &mut GraphRunContext,
    overrides: SourceProcessOverrides,
    runtime_factory: &dyn signal_processing::AppManagerFactory,
) -> Result<LiveRun, Vec<ProcessingGraphError>> {
    start_live_inner(compiled, ctx, overrides, runtime_factory)
}
