use signal_capture_session::CaptureSourceMetadataError;

use crate::node_support::PersistedStateError;

/// Failure to construct one graph node's processing-runtime implementation.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeMaterializationError {
    /// The node's persisted state could not be decoded.
    #[error(transparent)]
    State(#[from] PersistedStateError),
    /// The node configuration is not valid for construction.
    #[error("{0}")]
    Configuration(String),
    /// A lower-level typed configuration failure with node-owned context.
    #[error("{context}: {source}")]
    ConfigurationContext {
        /// Node-owned description of the failed configuration operation.
        context: String,
        /// Concrete lower-level configuration cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A required run-scoped input or resource is unavailable.
    #[error("{0}")]
    Unavailable(String),
    /// A lower-level factory could not construct the processing node.
    #[error("{0}")]
    Construction(String),
    /// A lower-level factory exposed a typed construction failure.
    #[error("{source}")]
    ConstructionSource {
        /// Concrete lower-level construction cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A lower-level typed construction failure with node-owned context.
    #[error("{context}: {source}")]
    ConstructionContext {
        /// Node-owned description of the failed construction operation.
        context: String,
        /// Concrete lower-level construction cause.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A materializer was invoked through an unsupported capability path.
    #[error("{0}")]
    Contract(String),
}

impl RuntimeMaterializationError {
    /// Classifies invalid node configuration.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    /// Retains a lower-level typed configuration failure with node-owned context.
    pub fn configuration_context(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::ConfigurationContext {
            context: context.into(),
            source: Box::new(source),
        }
    }

    /// Classifies an unavailable run-scoped input or resource.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// Adapts a lower-level construction diagnostic whose owner still exposes text.
    pub fn construction(message: impl Into<String>) -> Self {
        Self::Construction(message.into())
    }

    /// Retains a lower-level typed construction failure.
    pub fn construction_source(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::ConstructionSource {
            source: Box::new(source),
        }
    }

    /// Retains a lower-level typed construction failure with node-owned context.
    pub fn construction_context(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::ConstructionContext {
            context: context.into(),
            source: Box::new(source),
        }
    }

    /// Classifies use of an unsupported materialization path.
    pub fn contract(message: impl Into<String>) -> Self {
        Self::Contract(message.into())
    }
}

#[cfg(test)]
mod runtime_materialization_error_tests {
    use std::error::Error;

    use serde_json::json;

    use super::RuntimeMaterializationError;
    use crate::node_support::parse_state;

    #[test]
    fn persisted_state_causes_survive_materialization() {
        let state_error = parse_state::<u64>(&json!("not a number")).unwrap_err();
        let error = RuntimeMaterializationError::from(state_error);

        assert!(matches!(error, RuntimeMaterializationError::State(_)));
        assert!(error.source().is_some());
    }

    #[derive(Debug, thiserror::Error)]
    #[error("controlled construction failure")]
    struct ControlledConstructionFailure;

    #[test]
    fn typed_construction_causes_survive_materialization() {
        let error = RuntimeMaterializationError::construction_source(ControlledConstructionFailure);

        assert!(matches!(
            error,
            RuntimeMaterializationError::ConstructionSource { .. }
        ));
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("controlled construction failure")
        );
    }

    #[test]
    fn typed_configuration_causes_survive_materialization_context() {
        let error = RuntimeMaterializationError::configuration_context(
            "could not configure collector lane",
            ControlledConstructionFailure,
        );

        assert!(matches!(
            error,
            RuntimeMaterializationError::ConfigurationContext { .. }
        ));
        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("controlled construction failure")
        );
    }
}

/// Failure exposed by a node's generic live-capture capability.
#[derive(Debug, thiserror::Error)]
pub enum LiveCaptureFeatureError {
    /// The node's persisted state could not be decoded or encoded.
    #[error(transparent)]
    State(#[from] PersistedStateError),
    /// Lazy capture-source metadata could not supply live-acquisition configuration.
    #[error(transparent)]
    Metadata(#[from] CaptureSourceMetadataError),
    /// The node's capture or trigger configuration is invalid.
    #[error("{0}")]
    Configuration(String),
    /// A requested edit is invalid for the node's current state.
    #[error("{0}")]
    Edit(String),
    /// The provider exposed an internally inconsistent live-capture contract.
    #[error("{0}")]
    InvalidProvider(String),
}

impl LiveCaptureFeatureError {
    /// Classifies a node-owned capture or trigger configuration diagnostic.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    /// Classifies a node-owned live-capture edit diagnostic.
    pub fn edit(message: impl Into<String>) -> Self {
        Self::Edit(message.into())
    }

    /// Classifies an internally inconsistent provider contract.
    pub fn invalid_provider(message: impl Into<String>) -> Self {
        Self::InvalidProvider(message.into())
    }
}

/// Failure exposed by a node's generic timeline capability.
#[derive(Debug, thiserror::Error)]
pub enum TimelineFeatureError {
    /// The node's persisted state could not be decoded or encoded.
    #[error(transparent)]
    State(#[from] PersistedStateError),
    /// An edit addressed a marker identity not owned by the node.
    #[error("unknown timeline marker '{id}'")]
    UnknownMarker {
        /// Node-local marker identity supplied by the host.
        id: String,
    },
    /// An edit addressed a reference control not owned by the node.
    #[error("unknown timeline reference '{id}'")]
    UnknownReference {
        /// Node-local reference-control identity supplied by the host.
        id: String,
    },
    /// A plugin could provide only an unclassified timeline diagnostic.
    #[error("{0}")]
    Diagnostic(String),
}

impl TimelineFeatureError {
    /// Classifies an edit addressed to an unknown node-local marker.
    pub fn unknown_marker(id: impl Into<String>) -> Self {
        Self::UnknownMarker { id: id.into() }
    }

    /// Classifies an edit addressed to an unknown node-local reference control.
    pub fn unknown_reference(id: impl Into<String>) -> Self {
        Self::UnknownReference { id: id.into() }
    }

    /// Adapts a plugin which can expose only a timeline diagnostic.
    pub fn diagnostic(message: impl Into<String>) -> Self {
        Self::Diagnostic(message.into())
    }
}
