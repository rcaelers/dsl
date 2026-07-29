inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::BinaryDecoder,
        super::super::parallel_decoder::ParallelDecoderBuilder,
    >("org.logicconduit.graph-node.decoders.binary-decoder/v1").requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn binary_decoder_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.decoders.binary-decoder/v1",
        );
    }
}
