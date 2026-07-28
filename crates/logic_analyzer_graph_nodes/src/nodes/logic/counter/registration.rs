inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::Counter,
        super::builder::CounterBuilder,
    >("org.logicconduit.graph-node.logic.counter/v1").requiring_payloads(&[
        "org.logicconduit.number-sample/v1",
        "org.logicconduit.trigger/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn counter_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.counter/v1",
        );
    }
}
