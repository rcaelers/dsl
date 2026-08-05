inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::TestUartSource,
        super::builder::TestUartSourceBuilder,
        super::builder::TestUartSourceBuilder,
    >("org.logicconduit.graph-node.sources.test-uart-source/v1")
    .with_capture_source::<super::builder::TestUartSourceBuilder>()
    .with_presentation::<super::builder::TestUartSourceBuilder>()
    .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn test_uart_source_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.sources.test-uart-source/v1",
        );
    }
}
