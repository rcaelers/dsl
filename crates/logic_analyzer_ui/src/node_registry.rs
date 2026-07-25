use logic_analyzer_graph_api::node::graph_node_registrations;
use node_graph::NodeTypeRegistry;

pub fn build_node_registry() -> NodeTypeRegistry {
    let mut registry = NodeTypeRegistry::new();
    for registration in graph_node_registrations() {
        assert!(
            registry.category_of(registration.name()).is_none(),
            "graph-node inventory definition '{}' conflicts with an explicit catalog entry",
            registration.name()
        );
        registration.apply_node(&mut registry);
    }
    registry
}
