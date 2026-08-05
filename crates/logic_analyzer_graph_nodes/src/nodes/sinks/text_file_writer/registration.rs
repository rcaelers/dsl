inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::TextFileWriter,
        super::builder::TextFileWriterBuilder,
        super::builder::TextFileWriterBuilder,
    >("org.logicconduit.graph-node.sinks.text-file-writer/v1")
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
