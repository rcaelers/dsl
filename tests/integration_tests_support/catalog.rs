use logic_analyzer_graph_capabilities::node::GraphNodeSemantics;
use logic_analyzer_graph_registry::{GraphNodeRegistration, graph_node_registrations};
use node_graph::api::NodeTypeRegistry;

fn registration(stable_id: &str) -> &'static GraphNodeRegistration {
    logic_analyzer_graph_nodes::link();
    graph_node_registrations()
        .into_iter()
        .find(|registration| registration.stable_id() == stable_id)
        .unwrap_or_else(|| panic!("graph node '{stable_id}' is not registered"))
}

pub(crate) fn build_registry() -> NodeTypeRegistry {
    logic_analyzer_graph_nodes::link();
    logic_analyzer_ui::build_node_registry()
}

pub(crate) fn node_name(stable_id: &str) -> &'static str {
    registration(stable_id).name()
}

pub(crate) fn node_semantics(stable_id: &str) -> Box<dyn GraphNodeSemantics> {
    registration(stable_id)
        .semantics()
        .unwrap_or_else(|| panic!("graph node '{stable_id}' has no graph semantics"))
}
