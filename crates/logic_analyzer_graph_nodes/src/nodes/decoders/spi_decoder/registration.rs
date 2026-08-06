inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::SpiDecoder>("org.logicconduit.graph-node.decoders.spi-decoder/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::SpiDecoderBuilder, super::builder::SpiDecoderBuilder>(
        "org.logicconduit.graph-node.decoders.spi-decoder/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::SpiDecoder>,
    )
    .with_presentation::<super::builder::SpiDecoderBuilder>()
    .requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.word/v1",
        "org.logicconduit.protocol-packet/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn spi_decoder_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.decoders.spi-decoder/v1",
        );
    }
}
