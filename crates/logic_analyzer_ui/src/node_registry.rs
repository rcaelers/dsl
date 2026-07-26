use logic_analyzer_graph_api::node::graph_node_registrations;
use node_graph::NodeTypeRegistry;

use crate::viewer_selection::LEGACY_VIEWER_NODE_ID;

pub fn build_node_registry() -> NodeTypeRegistry {
    let mut registry = NodeTypeRegistry::new();
    for registration in graph_node_registrations() {
        if registration.stable_id() == LEGACY_VIEWER_NODE_ID {
            continue;
        }
        assert!(
            registry.category_of(registration.name()).is_none(),
            "graph-node inventory definition '{}' conflicts with an explicit catalog entry",
            registration.name()
        );
        registration.apply_node(&mut registry);
    }
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
