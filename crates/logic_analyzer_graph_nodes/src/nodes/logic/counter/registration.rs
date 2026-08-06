inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::Counter>("org.logicconduit.graph-node.logic.counter/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::CounterBuilder, super::builder::CounterBuilder>(
        "org.logicconduit.graph-node.logic.counter/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::Counter>,
    ).requiring_payloads(&[
        "org.logicconduit.number-sample/v1",
        "org.logicconduit.trigger/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn counter_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.counter/v1",
        );
    }
}
