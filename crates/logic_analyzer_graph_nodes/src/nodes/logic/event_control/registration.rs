inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::EventControl,
        super::builder::EventControlBuilder,
        super::builder::EventControlBuilder,
    >("org.logicconduit.graph-node.logic.event-control/v1").requiring_payloads(&[
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
