inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::I2cDecoder,
        super::builder::I2cDecoderBuilder,
    >("org.logicconduit.graph-node.i2c-decoder/v1").requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.word/v1",
        "org.logicconduit.protocol-packet/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn i2c_decoder_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.i2c-decoder/v1",
        );
    }
}
