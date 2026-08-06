inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::Viewer>("org.logicconduit.graph-node.sinks.viewer/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::ViewerSubscriptionBuilder, super::builder::ViewerSubscriptionBuilder>(
        "org.logicconduit.graph-node.sinks.viewer/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::Viewer>,
    )
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn viewer_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.sinks.viewer/v1",
        );
    }
}
