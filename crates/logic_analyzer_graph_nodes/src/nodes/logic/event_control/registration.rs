inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::EventControl>("org.logicconduit.graph-node.logic.event-control/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::EventControlBuilder, super::builder::EventControlBuilder>(
        "org.logicconduit.graph-node.logic.event-control/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::EventControl>,
    ).requiring_payloads(&[
        "org.logicconduit.trigger/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn event_control_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.event-control/v1",
        );
    }
}
