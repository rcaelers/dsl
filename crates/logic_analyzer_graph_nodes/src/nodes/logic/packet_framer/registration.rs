inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::PacketFramer>("org.logicconduit.graph-node.logic.packet-framer/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::PacketFramerBuilder, super::builder::PacketFramerBuilder>(
        "org.logicconduit.graph-node.logic.packet-framer/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::PacketFramer>,
    )
    .with_presentation::<super::builder::PacketFramerBuilder>()
    .requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.protocol-packet/v1",
        "org.logicconduit.trigger/v1",
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn packet_framer_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.packet-framer/v1",
        );
    }
}
