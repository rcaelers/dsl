use std::collections::BTreeMap;

use logic_analyzer_graph_editor_registry::{
    GraphNodeEditorOverride, graph_node_editor_registrations,
};
use logic_analyzer_graph_registry::graph_node_registrations;
use node_graph::api::NodeTypeRegistry;

use crate::viewer_selection::LEGACY_VIEWER_NODE_ID;

/// Builds the UI node registry from validated graph-node registrations.
///
/// The obsolete viewer node is deliberately excluded because output selection is
/// now persisted as UI-owned state rather than represented by a graph node.
pub fn build_node_registry() -> NodeTypeRegistry {
    build_node_registry_with_editor_overrides(Vec::new())
}

pub(crate) fn build_node_registry_with_editor_overrides(
    editor_overrides: Vec<GraphNodeEditorOverride>,
) -> NodeTypeRegistry {
    let mut overrides_by_id = BTreeMap::new();
    for editor_override in editor_overrides {
        let stable_id = editor_override.stable_id().to_owned();
        assert!(
            overrides_by_id
                .insert(stable_id.clone(), editor_override)
                .is_none(),
            "duplicate graph-node editor override '{stable_id}'"
        );
    }
    let mut registry = NodeTypeRegistry::new();
    let mut feature_names = graph_node_registrations()
        .into_iter()
        .map(|registration| (registration.stable_id(), registration.name()))
        .collect::<BTreeMap<_, _>>();
    for registration in graph_node_editor_registrations() {
        let feature_name = feature_names
            .remove(registration.stable_id())
            .unwrap_or_else(|| {
                panic!(
                    "graph-node editor '{}' has no matching headless feature registration",
                    registration.stable_id()
                )
            });
        assert_eq!(
            registration.name(),
            feature_name,
            "graph-node editor and headless feature names differ for '{}'",
            registration.stable_id()
        );
        if registration.stable_id() == LEGACY_VIEWER_NODE_ID {
            continue;
        }
        assert!(
            registry.category_of(registration.name()).is_none(),
            "graph-node inventory definition '{}' conflicts with an explicit catalog entry",
            registration.name()
        );
        if let Some(editor_override) = overrides_by_id.remove(registration.stable_id()) {
            editor_override.apply(&mut registry);
        } else {
            registration.apply_node(&mut registry);
        }
    }
    assert!(
        overrides_by_id.is_empty(),
        "editor overrides reference unregistered graph-node features: {:?}",
        overrides_by_id.keys().collect::<Vec<_>>()
    );
    assert!(
        feature_names.is_empty(),
        "headless graph-node features have no editor registration: {:?}",
        feature_names.keys().collect::<Vec<_>>()
    );
    registry
}

#[cfg(test)]
mod node_registry_tests {
    use super::*;

    #[test]
    fn obsolete_viewer_node_is_not_offered_by_the_ui_catalog() {
        assert!(build_node_registry().category_of("Viewer").is_none());
    }
}
