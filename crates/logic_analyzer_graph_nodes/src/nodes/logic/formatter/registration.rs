inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::StringFormatter>("org.logicconduit.graph-node.logic.string-formatter/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::FormatterBuilder, super::builder::FormatterBuilder>(
        "org.logicconduit.graph-node.logic.string-formatter/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::StringFormatter>,
    ).requiring_payloads(&[
        "org.logicconduit.number-sample/v1",
        "org.logicconduit.text-sample/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn string_formatter_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.string-formatter/v1",
        );
    }
}
