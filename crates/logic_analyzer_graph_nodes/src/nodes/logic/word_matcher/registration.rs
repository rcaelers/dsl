inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::WordMatcher,
        super::builder::WordMatcherBuilder,
        super::builder::WordMatcherBuilder,
    >("org.logicconduit.graph-node.logic.word-matcher/v1").requiring_payloads(&[
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
