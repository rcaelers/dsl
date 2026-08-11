use logic_analyzer_graph_editor_registry::{
    GraphNodeEditorRegistration, graph_node_editor_registrations,
};
use node_graph::api::NodeTypeRegistry;

fn registrations() -> impl Iterator<Item = &'static GraphNodeEditorRegistration> {
    graph_node_editor_registrations().into_iter()
}

pub(crate) fn build_registry() -> NodeTypeRegistry {
    let mut registry = NodeTypeRegistry::new();
    for registration in registrations() {
        registration.apply_node(&mut registry);
    }
    registry
}
