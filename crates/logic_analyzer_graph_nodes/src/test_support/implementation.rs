use logic_analyzer_graph_api::node::GraphNodeRegistration;
use node_graph::NodeTypeRegistry;

fn registrations() -> impl Iterator<Item = &'static GraphNodeRegistration> {
    inventory::iter::<GraphNodeRegistration>.into_iter()
}

fn registration(stable_id: &str) -> &'static GraphNodeRegistration {
    registrations()
        .find(|registration| registration.stable_id() == stable_id)
        .unwrap_or_else(|| panic!("graph node '{stable_id}' is not registered"))
}

pub(crate) fn build_registry() -> NodeTypeRegistry {
    let mut registry = NodeTypeRegistry::new();
    for registration in registrations() {
        registration.apply_node(&mut registry);
    }
    registry
}

pub(crate) fn node_name(stable_id: &str) -> &'static str {
    registration(stable_id).name()
}
