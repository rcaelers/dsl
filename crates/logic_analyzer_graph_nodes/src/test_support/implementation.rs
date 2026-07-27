use logic_analyzer_graph_api::node::GraphNodeRegistration;
use node_graph::NodeTypeRegistry;

fn registrations() -> impl Iterator<Item = &'static GraphNodeRegistration> {
    inventory::iter::<GraphNodeRegistration>.into_iter()
}

pub(crate) fn build_registry() -> NodeTypeRegistry {
    let mut registry = NodeTypeRegistry::new();
    for registration in registrations() {
        registration.apply_node(&mut registry);
    }
    registry
}
