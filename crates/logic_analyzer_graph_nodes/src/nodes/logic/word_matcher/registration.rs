inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::WordMatcher>("org.logicconduit.graph-node.logic.word-matcher/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::WordMatcherBuilder, super::builder::WordMatcherBuilder>(
        "org.logicconduit.graph-node.logic.word-matcher/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::WordMatcher>,
    ).requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.trigger/v1",
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn word_matcher_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.word-matcher/v1",
        );
    }
}
