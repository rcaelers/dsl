inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::Viewer,
        super::builder::ViewerSubscriptionBuilder,
        super::builder::ViewerSubscriptionBuilder,
    >("org.logicconduit.graph-node.sinks.viewer/v1")
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
