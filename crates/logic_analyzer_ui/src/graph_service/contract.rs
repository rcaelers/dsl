use std::fmt;
use std::time::Duration;

use logic_analyzer_graph_plan::{
    CollectedOutputSubscription, CollectedTableSubscription, ProcessingGraph,
    ProcessingGraphError as CompileError, SamplingOverlayCandidate,
};
use logic_analyzer_graph_runtime::{
    ApplyError, ApplySummary, GraphRunContext, SourceReadinessRegistry,
};
use node_graph::api::{GraphState, NodeId};
use signal_derived::PersistentStoreConfig;
use signal_runtime::{ConfigurationBoundary, DisconnectEvent, NodeFailure};

pub(crate) type CachedDataLoader<'a> =
    dyn FnMut(&GraphState, &mut GraphRunContext) -> Result<bool, Vec<CompileError>> + 'a;

/// Classified terminal failure from an active graph run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GraphRunFailure {
    Busy,
    Compilation(String),
    Execution(String),
    Node(String),
    Artifact(String),
    Cache(String),
    Transport(String),
}

impl fmt::Display for GraphRunFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => formatter.write_str("the graph worker already has an active run"),
            Self::Compilation(message) => write!(formatter, "graph compilation failed: {message}"),
            Self::Execution(message) => write!(formatter, "graph execution failed: {message}"),
            Self::Node(message) => write!(formatter, "graph node execution failed: {message}"),
            Self::Artifact(message) => {
                write!(formatter, "graph artifact replication failed: {message}")
            }
            Self::Cache(message) => write!(formatter, "graph cache preparation failed: {message}"),
            Self::Transport(message) => {
                write!(formatter, "graph worker transport failed: {message}")
            }
        }
    }
}

pub(crate) trait GraphRun {
    fn persistent_cache_configs(&self) -> Vec<PersistentStoreConfig>;

    fn sampling_overlays(&self) -> &[SamplingOverlayCandidate];

    fn output_subscriptions(&self) -> &[CollectedOutputSubscription];

    fn table_subscriptions(&self) -> &[CollectedTableSubscription];

    fn source_readiness(&self) -> &SourceReadinessRegistry;

    fn is_finished(&self) -> bool;

    fn is_stopping(&self) -> bool;

    fn stop(&mut self);

    fn pump(&mut self, budget: usize);

    fn pump_for(&mut self, budget: usize, _max_duration: Duration) {
        self.pump(budget);
    }

    fn wait(&mut self) {
        while !self.is_finished() {
            self.pump_for(256, Duration::from_millis(4));
            std::thread::yield_now();
        }
    }

    fn progress(&self) -> Vec<(NodeId, u64)>;

    fn take_disconnected(&self) -> Vec<(Option<NodeId>, DisconnectEvent)>;

    fn take_node_failures(&mut self) -> Vec<(Option<NodeId>, NodeFailure)> {
        Vec::new()
    }

    fn take_failure(&mut self) -> Option<GraphRunFailure> {
        None
    }

    fn apply_processing_graph(
        &mut self,
        _graph: ProcessingGraph,
        _boundary: Option<ConfigurationBoundary>,
    ) -> Result<ApplySummary, ApplyError> {
        Err(ApplyError::Apply(
            "this graph execution host does not support live plan updates".into(),
        ))
    }

    fn synchronize_cached_data(
        &mut self,
        _graph: &GraphState,
        _load: &mut CachedDataLoader<'_>,
    ) -> Result<bool, Vec<CompileError>> {
        Ok(false)
    }
}
