inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::SigrokDecoderDefinition,
        super::builder::SigrokDecoderBuilder,
    >("org.logicconduit.graph-node.sigrok-decoder/v1").requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.word/v1",
        "org.logicconduit.protocol-packet/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    use egui::Pos2;
    use logic_analyzer_graph_api::node::RuntimeBuilder;
    use logic_analyzer_processing::nodes::decoders::sigrok_decoder::discover_sigrok_decoder;
    use node_graph::{NodeDef, NodeGraphWidget, NodeTypeRegistry};

    use super::super::builder::SigrokDecoderBuilder;
    use super::super::definition::{SigrokDecoderDefinition, SigrokDecoderState};

    #[test]
    fn checked_in_logic_decoder_registration_contract_uses_discovered_metadata() {
        let decoder_root = test_decoder_root();
        let descriptor = discover_sigrok_decoder(&decoder_root, "test_logic").unwrap();
        let mut state = super::super::definition::SigrokDecoderState::from_descriptor(
            decoder_root,
            &descriptor,
        );
        for channel in &mut state.channels {
            if matches!(channel.id.as_str(), "mosi" | "cs") {
                channel.enabled.value = true;
            }
        }
        crate::nodes::test_support::assert_node_registration_contract_with_state(
            "org.logicconduit.graph-node.sigrok-decoder/v1",
            Some(serde_json::to_value(state).unwrap()),
        );
    }

    #[test]
    fn checked_in_stacked_decoder_registration_contract_accepts_protocol_packets() {
        let decoder_root = test_decoder_root();
        let descriptor = discover_sigrok_decoder(&decoder_root, "test_stacked").unwrap();
        assert_eq!(descriptor.inputs, ["test_logic"]);
        let state = SigrokDecoderState::from_descriptor(decoder_root, &descriptor);
        crate::nodes::test_support::assert_node_registration_contract_with_state(
            "org.logicconduit.graph-node.sigrok-decoder/v1",
            Some(serde_json::to_value(state).unwrap()),
        );
    }

    #[test]
    fn graph_connection_contracts_follow_declared_protocol_ids() {
        let decoder_root = test_decoder_root();
        let spi = SigrokDecoderState::from_descriptor(
            decoder_root.clone(),
            &discover_sigrok_decoder(&decoder_root, "test_logic").unwrap(),
        );
        let spiflash_descriptor = discover_sigrok_decoder(&decoder_root, "test_stacked").unwrap();
        let spiflash = SigrokDecoderState::from_descriptor(decoder_root, &spiflash_descriptor);
        let mut registry = NodeTypeRegistry::new();
        registry.register::<SigrokDecoderDefinition>();
        let mut widget = NodeGraphWidget::new(registry);
        let producer = widget
            .add_node_at(SigrokDecoderDefinition::name(), Pos2::ZERO)
            .unwrap();
        let consumer = widget
            .add_node_at(SigrokDecoderDefinition::name(), Pos2::new(100.0, 0.0))
            .unwrap();
        assert!(widget.set_node_state(producer, serde_json::to_value(spi).unwrap()));
        assert!(widget.set_node_state(consumer, serde_json::to_value(spiflash).unwrap()));
        let producer_node = &widget.graph().nodes[&producer];
        let consumer_node = &widget.graph().nodes[&consumer];
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
        let builder = SigrokDecoderBuilder;
        let offered = builder.offered_connection_contracts(output, &producer_node.state);
        let accepted = builder.accepted_connection_contracts(input, &consumer_node.state);
        assert_eq!(offered, ["test_logic"]);
        assert_eq!(accepted, ["test_logic"]);
        assert!(offered.iter().any(|contract| accepted.contains(contract)));
    }

    #[test]
    fn previous_saved_state_gains_protocol_contracts_with_a_warning() {
        let decoder_root = test_decoder_root();
        let descriptor = discover_sigrok_decoder(&decoder_root, "test_logic").unwrap();
        let mut state = SigrokDecoderState::from_descriptor(decoder_root.clone(), &descriptor);
        state.schema_version = 1;
        state.protocol_outputs.clear();
        state.catalog.search_paths = decoder_root.display().to_string();

        let mut registry = NodeTypeRegistry::new();
        registry.register::<SigrokDecoderDefinition>();
        let mut widget = NodeGraphWidget::new(registry);
        let node = widget
            .add_node_at(SigrokDecoderDefinition::name(), Pos2::ZERO)
            .unwrap();
        assert!(widget.set_node_state(node, serde_json::to_value(state).unwrap()));
        let migrated: SigrokDecoderState =
            serde_json::from_value(widget.graph().nodes[&node].state.clone()).unwrap();
        assert_eq!(migrated.schema_version, 3);
        assert_eq!(migrated.protocol_outputs, ["test_logic"]);
        assert!(
            widget.graph().nodes[&node]
                .badge
                .as_ref()
                .is_some_and(|badge| badge.text.contains("protocol connection contracts"))
        );
    }

    fn test_decoder_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data/sigrok_decoders")
    }
}
