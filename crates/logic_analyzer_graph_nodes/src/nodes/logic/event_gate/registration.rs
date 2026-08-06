inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::EventGate>("org.logicconduit.graph-node.logic.event-gate/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::EventGateBuilder, super::builder::EventGateBuilder>(
        "org.logicconduit.graph-node.logic.event-gate/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::EventGate>,
    ).requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.trigger/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn event_gate_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.event-gate/v1",
        );
    }
}
