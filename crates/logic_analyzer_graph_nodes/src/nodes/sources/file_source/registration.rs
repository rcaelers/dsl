inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::DslFileSource>("org.logicconduit.graph-node.sources.dsl-file-source/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::FileSourceBuilder, super::builder::FileSourceBuilder>(
        "org.logicconduit.graph-node.sources.dsl-file-source/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::DslFileSource>,
    )
        .with_capture_source::<super::builder::FileSourceBuilder>()
        .with_presentation::<super::builder::FileSourceBuilder>()
        .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    use node_graph::NodeDef;

    use super::super::definition::DslFileSource;

    #[test]
    fn dsl_file_source_registration_contract_is_self_consistent() {
        let mut state = serde_json::to_value(DslFileSource::state()).unwrap();
        state["channel_names"] = serde_json::json!(["Clock"]);
        crate::nodes::test_support::assert_node_registration_contract_without_runtime_with_state(
            "org.logicconduit.graph-node.sources.dsl-file-source/v1",
            state,
        );
    }
}
