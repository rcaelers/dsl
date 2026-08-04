inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::runnable::<
        super::definition::WordFieldExtractor,
        super::builder::WordFieldExtractorBuilder,
    >("org.logicconduit.graph-node.logic.word-field-extractor/v1").requiring_payloads(&[
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn word_field_extractor_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.word-field-extractor/v1",
        );
    }
}
