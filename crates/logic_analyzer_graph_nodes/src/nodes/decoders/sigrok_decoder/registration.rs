inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::SigrokDecoderDefinition>("org.logicconduit.graph-node.decoders.sigrok-decoder/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::SigrokDecoderBuilder, super::builder::SigrokDecoderBuilder>(
        "org.logicconduit.graph-node.decoders.sigrok-decoder/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::SigrokDecoderDefinition>,
    ).requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.word/v1",
        "org.logicconduit.protocol-packet/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    use logic_analyzer_graph_capabilities::node::GraphNodeSemantics;
    use node_graph::api::{GraphDocumentBuilder, NodeDef, NodeTypeRegistry, SocketDirection};

    use super::super::builder::SigrokDecoderBuilder;
    use super::super::definition::{SigrokDecoderDefinition, SigrokDecoderState};
    use crate::nodes::test_support::{
        test_sigrok_logic_descriptor, test_sigrok_stacked_descriptor,
    };

    #[test]
    fn logic_decoder_registration_contract_uses_saved_metadata() {
        let decoder_root = std::path::PathBuf::from("virtual/sigrok-decoders");
        let descriptor = test_sigrok_logic_descriptor();
        let state = super::super::definition::SigrokDecoderState::from_descriptor(
            decoder_root,
            &descriptor,
        );
        crate::nodes::test_support::assert_node_registration_contract_without_runtime_with_state(
            "org.logicconduit.graph-node.decoders.sigrok-decoder/v1",
            serde_json::to_value(state).unwrap(),
        );
    }

    #[test]
    fn stacked_decoder_registration_contract_accepts_protocol_packets() {
        let decoder_root = std::path::PathBuf::from("virtual/sigrok-decoders");
        let descriptor = test_sigrok_stacked_descriptor();
        assert_eq!(descriptor.inputs, ["test_logic"]);
        let state = SigrokDecoderState::from_descriptor(decoder_root, &descriptor);
        crate::nodes::test_support::assert_node_registration_contract_without_runtime_with_state(
            "org.logicconduit.graph-node.decoders.sigrok-decoder/v1",
            serde_json::to_value(state).unwrap(),
        );
    }

    #[test]
    fn graph_connection_contracts_follow_declared_protocol_ids() {
        let decoder_root = std::path::PathBuf::from("virtual/sigrok-decoders");
        let spi = SigrokDecoderState::from_descriptor(
            decoder_root.clone(),
            &test_sigrok_logic_descriptor(),
        );
        let spiflash_descriptor = test_sigrok_stacked_descriptor();
        let spiflash = SigrokDecoderState::from_descriptor(decoder_root, &spiflash_descriptor);
        let mut registry = NodeTypeRegistry::new();
        registry.register::<SigrokDecoderDefinition>();
        let mut document = GraphDocumentBuilder::new(registry);
        let producer = document.add_node(SigrokDecoderDefinition::name()).unwrap();
        let consumer = document.add_node(SigrokDecoderDefinition::name()).unwrap();
        assert!(document.set_node_state(producer, serde_json::to_value(spi).unwrap()));
        assert!(document.set_node_state(consumer, serde_json::to_value(spiflash).unwrap()));
        let producer_node = &document.graph().nodes[&producer];
        let consumer_node = &document.graph().nodes[&consumer];
        let output = producer_node
            .outputs
            .iter()
            .find(|socket| socket.schema_id == "packets")
            .unwrap();
        let input = consumer_node
            .inputs
            .iter()
            .find(|socket| socket.schema_id == "protocol_packets")
            .unwrap();
        let builder = SigrokDecoderBuilder::default();
        let offered = builder.offered_connection_contracts(
            output.reference(SocketDirection::Output, 0),
            &producer_node.state,
        );
        let accepted = builder.accepted_connection_contracts(
            input.reference(SocketDirection::Input, 0),
            &consumer_node.state,
        );
        assert_eq!(offered, ["test_logic"]);
        assert_eq!(accepted, ["test_logic"]);
        assert!(offered.iter().any(|contract| accepted.contains(contract)));
    }
}
