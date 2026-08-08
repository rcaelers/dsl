use logic_analyzer_graph_capabilities::node::RuntimeMaterializationError;
use logic_analyzer_graph_plan::ProcessingGraphError;
use node_graph_document::NodeId;
use signal_runtime::PipelineError;

/// Failure while materializing or starting an already-lowered processing graph.
#[derive(Debug, thiserror::Error)]
pub enum GraphRuntimeError {
    /// One node's runtime capability could not construct its processing implementation.
    #[error("could not materialize graph node n{}: {source}", node.0)]
    Materialization {
        /// Graph-document node being materialized.
        node: NodeId,
        /// Capability-owned construction failure.
        #[source]
        source: RuntimeMaterializationError,
    },
    /// The lowered plan violates a runtime precondition.
    #[error("invalid runtime plan for graph node n{}: {message}", node.0)]
    InvalidPlan {
        /// Graph-document node whose plan is invalid.
        node: NodeId,
        /// Runtime-owned contract diagnostic.
        message: String,
    },
    /// Pipeline creation, registration, or startup failed.
    #[error("graph runtime failed: {0}")]
    Pipeline(#[from] PipelineError),
    /// Pipeline registration failed for one graph node.
    #[error("graph runtime failed for node n{}: {source}", node.0)]
    NodePipeline {
        /// Graph-document node being registered.
        node: NodeId,
        /// Typed stream-runtime failure.
        #[source]
        source: PipelineError,
    },
}

impl GraphRuntimeError {
    /// Returns graph-node context when the failure belongs to one node.
    pub fn node(&self) -> Option<NodeId> {
        match self {
            Self::Materialization { node, .. }
            | Self::InvalidPlan { node, .. }
            | Self::NodePipeline { node, .. } => Some(*node),
            Self::Pipeline(_) => None,
        }
    }

    pub(crate) fn materialization(node: NodeId, source: RuntimeMaterializationError) -> Self {
        Self::Materialization { node, source }
    }

    pub(crate) fn invalid_plan(node: NodeId, message: impl Into<String>) -> Self {
        Self::InvalidPlan {
            node,
            message: message.into(),
        }
    }

    pub(crate) fn node_pipeline(node: NodeId, source: PipelineError) -> Self {
        Self::NodePipeline { node, source }
    }
}

/// Error produced while reconciling an active graph run.
#[derive(Debug)]
pub enum ApplyError {
    /// The edited graph did not lower; the active run is untouched.
    Compile(Vec<ProcessingGraphError>),
    /// The edit requires stopping and starting a new run.
    NeedsFullRestart(String),
    /// Runtime reconciliation failed after it began.
    Apply(String),
    /// A newly added or restarted node could not be materialized.
    Materialization {
        /// Graph-document node being materialized.
        node: NodeId,
        /// Capability-owned construction failure.
        source: RuntimeMaterializationError,
    },
    /// Stream-pipeline supervision rejected a live reconciliation operation.
    Runtime(PipelineError),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(errors) => {
                write!(
                    formatter,
                    "edited graph has {} compile error(s)",
                    errors.len()
                )
            }
            Self::NeedsFullRestart(message) => {
                write!(formatter, "live edit requires a full restart: {message}")
            }
            Self::Apply(message) => write!(formatter, "could not apply live edit: {message}"),
            Self::Materialization { node, source } => {
                write!(
                    formatter,
                    "could not materialize graph node n{}: {source}",
                    node.0
                )
            }
            Self::Runtime(error) => write!(formatter, "could not apply live edit: {error}"),
        }
    }
}

impl std::error::Error for ApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Materialization { source, .. } => Some(source),
            Self::Runtime(error) => Some(error),
            Self::Compile(_) | Self::NeedsFullRestart(_) | Self::Apply(_) => None,
        }
    }
}

#[cfg(test)]
mod graph_runtime_error_tests {
    use std::error::Error;

    use logic_analyzer_graph_capabilities::node::RuntimeMaterializationError;
    use node_graph_document::NodeId;

    use super::GraphRuntimeError;

    #[derive(Debug, thiserror::Error)]
    #[error("controlled node construction failure")]
    struct ControlledConstructionFailure;

    #[test]
    fn graph_node_context_does_not_flatten_the_materialization_source() {
        let materialization =
            RuntimeMaterializationError::construction_source(ControlledConstructionFailure);
        let error = GraphRuntimeError::materialization(NodeId(7), materialization);

        assert_eq!(error.node(), Some(NodeId(7)));
        let materialization = error.source().expect("materialization source");
        assert_eq!(
            materialization.source().map(ToString::to_string).as_deref(),
            Some("controlled node construction failure")
        );
    }
}
