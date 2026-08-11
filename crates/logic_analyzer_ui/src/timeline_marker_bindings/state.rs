use std::collections::HashMap;

use logic_analyzer_graph_capabilities::node_support::{
    TimelineMarkerReferenceBindingDescriptor, TimelineMarkerReferenceChoice,
};
use logic_analyzer_graph_compiler::DiscoveredTimelineMarker;
use logic_analyzer_viewer::TimelineMarker as ViewerTimelineMarker;
use node_graph::api::NodeId;

/// Owns the identity and diagnostic state for graph-backed timeline markers.
///
/// Marker owners always describe the most recent successful discovery. A marker-discovery failure
/// clears those owners, and repeated identical failures are reported only once. Reference-binding
/// failures are tracked independently because their discovery can fail while markers remain valid.
#[derive(Default)]
pub(crate) struct TimelineMarkerBindings {
    owners: HashMap<String, (NodeId, String)>,
    marker_error: Option<String>,
    reference_error: Option<String>,
}

impl TimelineMarkerBindings {
    pub(crate) fn replace_markers(
        &mut self,
        mut discovered: Vec<DiscoveredTimelineMarker>,
    ) -> Vec<ViewerTimelineMarker> {
        self.marker_error = None;
        discovered.sort_by(|left, right| {
            (left.owner_node.0, left.marker.id.as_str())
                .cmp(&(right.owner_node.0, right.marker.id.as_str()))
        });
        self.owners.clear();
        discovered
            .into_iter()
            .map(|discovered| {
                let id = format!("{}:{}", discovered.owner_node.0, discovered.marker.id);
                self.owners
                    .insert(id.clone(), (discovered.owner_node, discovered.marker.id));
                ViewerTimelineMarker {
                    id,
                    label: discovered.marker.name,
                    time_us: discovered.marker.timestamp_ns as f64 / 1_000.0,
                }
            })
            .collect()
    }

    pub(crate) fn record_marker_error(&mut self, error: String) -> bool {
        let is_new = self.marker_error.as_deref() != Some(&error);
        self.marker_error = Some(error);
        self.owners.clear();
        is_new
    }

    pub(crate) fn owner(&self, viewer_id: &str) -> Option<(NodeId, String)> {
        self.owners.get(viewer_id).cloned()
    }

    pub(crate) fn clear_reference_error(&mut self) {
        self.reference_error = None;
    }

    pub(crate) fn record_reference_error(&mut self, error: String) -> bool {
        let is_new = self.reference_error.as_deref() != Some(&error);
        self.reference_error = Some(error);
        is_new
    }

    pub(crate) fn reference_binding_is_synchronized(
        binding: &TimelineMarkerReferenceBindingDescriptor,
        choices: &[TimelineMarkerReferenceChoice],
    ) -> bool {
        if binding.choices != choices {
            return false;
        }
        let selected_timestamp = binding.selected.and_then(|selected| {
            choices
                .iter()
                .find(|choice| choice.reference == selected)
                .map(|choice| choice.timestamp_ns)
        });
        selected_timestamp
            .map(|timestamp_ns| timestamp_ns == binding.timestamp_ns)
            .unwrap_or(binding.selected.is_none())
    }
}
