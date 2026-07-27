inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::DsLogicU3Pro16,
        super::builder::DsLogicU3Pro16Builder,
    >("org.logicconduit.graph-node.dslogic-u3pro16/v1")
    .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn dslogic_u3pro16_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract_without_runtime(
            "org.logicconduit.graph-node.dslogic-u3pro16/v1",
        );
    }
}
