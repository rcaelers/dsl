inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::ParallelDecoder,
        super::builder::ParallelDecoderBuilder,
        super::builder::ParallelDecoderBuilder,
    >("org.logicconduit.graph-node.decoders.parallel-decoder/v1")
    .with_presentation::<super::builder::ParallelDecoderBuilder>()
    .requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.word/v1",
    ])
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::compatibility::BinaryDecoder,
        super::builder::ParallelDecoderBuilder,
        super::builder::ParallelDecoderBuilder,
    >("org.logicconduit.graph-node.decoders.binary-decoder/v1")
    .with_presentation::<super::builder::ParallelDecoderBuilder>()
    .requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn parallel_decoder_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.decoders.parallel-decoder/v1",
        );
    }

    #[test]
    fn binary_decoder_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.decoders.binary-decoder/v1",
        );
    }
}
