inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::Viewer,
        super::builder::ViewerSubscriptionBuilder,
    >("org.logicconduit.graph-node.viewer/v1")
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn viewer_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.viewer/v1",
        );
    }
}
