inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::CsvWriter>("org.logicconduit.graph-node.sinks.csv-writer/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::CsvWriterBuilder, super::builder::CsvWriterBuilder>(
        "org.logicconduit.graph-node.sinks.csv-writer/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::CsvWriter>,
    ).requiring_payloads(&[
        "org.logicconduit.text-sample/v1",
        "org.logicconduit.word/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn csv_writer_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.sinks.csv-writer/v1",
        );
    }
}
