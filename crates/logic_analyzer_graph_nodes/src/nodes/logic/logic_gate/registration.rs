inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::LogicGate,
        super::builder::LogicGateBuilder,
    >("org.logicconduit.graph-node.logic.logic-gate/v1")
    .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn logic_gate_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.logic-gate/v1",
        );
    }
}
