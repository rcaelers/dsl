use logic_analyzer_graph_registry::{GraphNodeRegistration, graph_node_registrations};
use node_graph::NodeTypeRegistry;

fn registrations() -> impl Iterator<Item = &'static GraphNodeRegistration> {
    graph_node_registrations().into_iter()
}

pub(crate) fn build_registry() -> NodeTypeRegistry {
    let mut registry = NodeTypeRegistry::new();
    for registration in registrations() {
        registration.apply_node(&mut registry);
    }
    registry
}
