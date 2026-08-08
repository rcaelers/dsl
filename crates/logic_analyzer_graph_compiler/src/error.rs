use std::error::Error;
use std::fmt;

use logic_analyzer_graph_capabilities::node::TimelineFeatureError;
use node_graph_document::NodeId;

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
                write!(formatter, "timeline-marker node {owner_node:?} no longer exists")
            }
            Self::ReferenceOwnerMissing { owner_node } => write!(
                formatter,
                "timeline-reference node {owner_node:?} no longer exists"
            ),
            Self::FeatureUnavailable { definition_name } => {
                write!(formatter, "no timeline feature is registered for {definition_name}")
            }
            Self::UnsupportedMarkerEdit { owner_title } => {
                write!(formatter, "{owner_title} does not support this timeline-marker edit")
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

    use serde_json::json;

    use logic_analyzer_graph_capabilities::node::TimelineFeatureError;
    use logic_analyzer_graph_capabilities::node_support::PersistedStateError;
    use node_graph_document::NodeId;

    use super::TimelineOperationError;

    #[test]
    fn feature_failure_retains_the_json_codec_cause() {
        let codec =
            serde_json::from_value::<usize>(json!("invalid")).expect_err("state must be invalid");
        let error = TimelineOperationError::feature(
            NodeId(7),
            "Marker",
            TimelineFeatureError::from(PersistedStateError::Decode(codec)),
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
