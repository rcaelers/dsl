inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::TextFileWriter>("org.logicconduit.graph-node.sinks.text-file-writer/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::TextFileWriterBuilder, super::builder::TextFileWriterBuilder>(
        "org.logicconduit.graph-node.sinks.text-file-writer/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::TextFileWriter>,
    )
    .requiring_payloads(&["org.logicconduit.text-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn text_file_writer_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.sinks.text-file-writer/v1",
        );
    }
}
