use std::error::Error;
use std::fmt;

use logic_analyzer_graph_capabilities::node::{LiveCaptureFeatureError, TimelineFeatureError};
use node_graph_document::NodeId;

/// Failure while discovering or editing node-owned live-capture capabilities.
#[derive(Debug)]
pub enum LiveCaptureOperationError {
    /// A concrete feature rejected discovery or an edit for its node.
    Feature {
        /// Stable graph-document identity of the feature owner.
        owner_node: NodeId,
        /// User-visible title used to contextualize the failure.
        owner_title: String,
        /// Typed failure reported by the node capability.
        source: LiveCaptureFeatureError,
    },
    /// A discovered feature violates the generic provider contract.
    InvalidFeature {
        /// Stable graph-document identity of the feature owner.
        owner_node: NodeId,
        /// User-visible title used to contextualize the failure.
        owner_title: String,
        /// Provider-contract invariant which was violated.
        message: String,
    },
    /// More than one enabled trigger configuration was discovered.
    MultipleTriggerConfigurations {
        /// Graph-document identities of the competing feature owners.
        source_nodes: Vec<NodeId>,
    },
    /// More than one enabled live-capture source was discovered.
    MultipleSources {
        /// Graph-document identities of the competing feature owners.
        source_nodes: Vec<NodeId>,
    },
    /// The source node receiving an edit no longer exists.
    OwnerMissing {
        /// Former graph-document identity of the feature owner.
        owner_node: NodeId,
    },
    /// The current registry has no live-capture capability for the node definition.
    FeatureUnavailable {
        /// Stable graph-document identity of the feature owner.
        owner_node: NodeId,
        /// Stable node-definition identity requested by the graph document.
        definition_name: String,
    },
    /// A live-capture capability did not implement the requested edit.
    UnsupportedEdit {
        /// Stable graph-document identity of the feature owner.
        owner_node: NodeId,
        /// User-visible title of the feature owner.
        owner_title: String,
    },
}

impl LiveCaptureOperationError {
    pub(crate) fn feature(
        owner_node: NodeId,
        owner_title: impl Into<String>,
        source: LiveCaptureFeatureError,
    ) -> Self {
        Self::Feature {
            owner_node,
            owner_title: owner_title.into(),
            source,
        }
    }

    /// Returns graph nodes responsible for the failure.
    pub fn source_nodes(&self) -> &[NodeId] {
        match self {
            Self::Feature { owner_node, .. }
            | Self::InvalidFeature { owner_node, .. }
            | Self::OwnerMissing { owner_node }
            | Self::FeatureUnavailable { owner_node, .. }
            | Self::UnsupportedEdit { owner_node, .. } => std::slice::from_ref(owner_node),
            Self::MultipleTriggerConfigurations { source_nodes }
            | Self::MultipleSources { source_nodes } => source_nodes,
        }
    }
}

impl fmt::Display for LiveCaptureOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Feature {
                owner_title,
                source,
                ..
            } => write!(formatter, "{owner_title}: {source}"),
            Self::InvalidFeature {
                owner_title,
                message,
                ..
            } => write!(formatter, "{owner_title}: {message}"),
            Self::MultipleTriggerConfigurations { .. } => write!(
                formatter,
                "multiple enabled trigger configurations are present; keep one capture source enabled"
            ),
            Self::MultipleSources { .. } => {
                formatter.write_str("the graph contains multiple live capture sources")
            }
            Self::OwnerMissing { owner_node } => {
                write!(
                    formatter,
                    "live capture source {owner_node:?} no longer exists"
                )
            }
            Self::FeatureUnavailable {
                definition_name, ..
            } => write!(
                formatter,
                "no live-capture feature is registered for {definition_name}"
            ),
            Self::UnsupportedEdit { owner_title, .. } => {
                write!(
                    formatter,
                    "{owner_title} does not support this live capture edit"
                )
            }
        }
    }
}

impl Error for LiveCaptureOperationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Feature { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Failure while discovering or editing node-owned timeline capabilities.
#[derive(Debug)]
pub enum TimelineOperationError {
    /// A concrete feature rejected discovery or an edit for its node.
    Feature {
        /// Stable graph-document identity of the feature owner.
        owner_node: NodeId,
        /// User-visible title used to contextualize the failure.
        owner_title: String,
        /// Typed failure reported by the node capability.
        source: TimelineFeatureError,
    },
    /// The graph node which owned an edited marker no longer exists.
    MarkerOwnerMissing {
        /// Former graph-document identity of the marker owner.
        owner_node: NodeId,
    },
    /// The graph node which owned an edited reference control no longer exists.
    ReferenceOwnerMissing {
        /// Former graph-document identity of the reference-control owner.
        owner_node: NodeId,
    },
    /// The current registry has no timeline capability for the node definition.
    FeatureUnavailable {
        /// Stable node-definition identity requested by the graph document.
        definition_name: String,
    },
    /// A timeline capability did not implement the requested marker edit.
    UnsupportedMarkerEdit {
        /// User-visible title of the feature owner.
        owner_title: String,
    },
    /// A timeline capability did not implement the requested reference edit.
    UnsupportedReferenceEdit {
        /// User-visible title of the feature owner.
        owner_title: String,
    },
}

impl fmt::Display for TimelineOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Feature {
                owner_title,
                source,
                ..
            } => write!(formatter, "{owner_title}: {source}"),
            Self::MarkerOwnerMissing { owner_node } => {
                write!(
                    formatter,
                    "timeline-marker node {owner_node:?} no longer exists"
                )
            }
            Self::ReferenceOwnerMissing { owner_node } => write!(
                formatter,
                "timeline-reference node {owner_node:?} no longer exists"
            ),
            Self::FeatureUnavailable { definition_name } => {
                write!(
                    formatter,
                    "no timeline feature is registered for {definition_name}"
                )
            }
            Self::UnsupportedMarkerEdit { owner_title } => {
                write!(
                    formatter,
                    "{owner_title} does not support this timeline-marker edit"
                )
            }
            Self::UnsupportedReferenceEdit { owner_title } => write!(
                formatter,
                "{owner_title} does not support this timeline-reference edit"
            ),
        }
    }
}

impl Error for TimelineOperationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Feature { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl TimelineOperationError {
    pub(crate) fn feature(
        owner_node: NodeId,
        owner_title: impl Into<String>,
        source: TimelineFeatureError,
    ) -> Self {
        Self::Feature {
            owner_node,
            owner_title: owner_title.into(),
            source,
        }
    }
}

#[cfg(test)]
mod error_tests {
    use std::error::Error as _;
    use std::io;

    use serde_json::json;

    use logic_analyzer_graph_capabilities::node::{LiveCaptureFeatureError, TimelineFeatureError};
    use logic_analyzer_graph_capabilities::node_support::PersistedStateError;
    use node_graph_document::NodeId;
    use signal_capture_session::CaptureSourceMetadataError;

    use super::{LiveCaptureOperationError, TimelineOperationError};

    #[test]
    fn live_feature_failure_retains_the_json_codec_cause() {
        let codec =
            serde_json::from_value::<usize>(json!("invalid")).expect_err("state must be invalid");
        let error = LiveCaptureOperationError::feature(
            NodeId(5),
            "Capture",
            LiveCaptureFeatureError::from(PersistedStateError::Decode(codec.into())),
        );

        let feature = error
            .source()
            .and_then(|source| source.downcast_ref::<LiveCaptureFeatureError>())
            .expect("live-capture feature source");
        assert!(
            feature
                .source()
                .is_some_and(|source| source.is::<serde_json::Error>()),
            "JSON codec source must survive"
        );
    }

    #[test]
    fn live_feature_failure_retains_the_capture_metadata_cause() {
        let error = LiveCaptureOperationError::feature(
            NodeId(6),
            "Capture",
            LiveCaptureFeatureError::from(CaptureSourceMetadataError::acquisition(
                io::Error::other("provider unavailable"),
            )),
        );

        let feature = error
            .source()
            .and_then(|source| source.downcast_ref::<LiveCaptureFeatureError>())
            .expect("live-capture feature source");
        assert!(
            matches!(
                feature,
                LiveCaptureFeatureError::Metadata(
                    CaptureSourceMetadataError::Acquisition(source)
                ) if source.downcast_ref::<io::Error>().is_some()
            ),
            "capture metadata cause must survive"
        );
    }

    #[test]
    fn feature_failure_retains_the_json_codec_cause() {
        let codec =
            serde_json::from_value::<usize>(json!("invalid")).expect_err("state must be invalid");
        let error = TimelineOperationError::feature(
            NodeId(7),
            "Marker",
            TimelineFeatureError::from(PersistedStateError::Decode(codec.into())),
        );

        let feature = error
            .source()
            .and_then(|source| source.downcast_ref::<TimelineFeatureError>())
            .expect("timeline feature source");
        assert!(
            feature
                .source()
                .is_some_and(|source| source.is::<serde_json::Error>()),
            "JSON codec source must survive"
        );
    }
}
