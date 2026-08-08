use signal_capture_session::CaptureSourceMetadataError;

use crate::node_support::PersistedStateError;

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
