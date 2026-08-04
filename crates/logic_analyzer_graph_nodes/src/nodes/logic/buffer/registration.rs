inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::runnable::<
        super::definition::Buffer,
        super::builder::BufferBuilder,
    >("org.logicconduit.graph-node.logic.buffer/v1").requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.number-sample/v1",
        "org.logicconduit.text-sample/v1",
        "org.logicconduit.trigger/v1",
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn buffer_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.buffer/v1",
        );
    }
}
