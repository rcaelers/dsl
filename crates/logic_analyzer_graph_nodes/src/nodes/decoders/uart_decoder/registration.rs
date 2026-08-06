inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::UartDecoder>("org.logicconduit.graph-node.decoders.uart-decoder/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::UartDecoderBuilder, super::builder::UartDecoderBuilder>(
        "org.logicconduit.graph-node.decoders.uart-decoder/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::UartDecoder>,
    )
    .with_presentation::<super::builder::UartDecoderBuilder>()
    .requiring_payloads(&[
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
