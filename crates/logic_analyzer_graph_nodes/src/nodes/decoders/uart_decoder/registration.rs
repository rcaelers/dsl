inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::UartDecoder,
        super::builder::UartDecoderBuilder,
    >("org.logicconduit.graph-node.decoders.uart-decoder/v1").requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.trigger/v1",
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn uart_decoder_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.decoders.uart-decoder/v1",
        );
    }
}
