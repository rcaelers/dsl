inventory::submit! {
    logic_analyzer_graph_api::node::GraphNodeRegistration::runnable::<
        super::definition::DslFileSource,
        super::builder::FileSourceBuilder,
    >("org.logicconduit.graph-node.dsl-file-source/v1")
        .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    use std::fs::File;
    use std::io::Write;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use node_graph::NodeDef;

    use super::super::definition::DslFileSource;

    #[test]
    fn dsl_file_source_registration_contract_accepts_a_checked_in_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("one-channel.dsl");
        let file = File::create(&path).unwrap();
        let mut archive = ZipWriter::new(file);
        archive
            .start_file("header", SimpleFileOptions::default())
            .unwrap();
        archive
            .write_all(
                b"total probes = 1\nsamplerate = 1 MHz\ntotal samples = 8\ntotal blocks = 1\nprobe0 = Clock\n",
            )
            .unwrap();
        archive
            .start_file("L-0/0", SimpleFileOptions::default())
            .unwrap();
        archive.write_all(&[0]).unwrap();
        archive.finish().unwrap();

        let mut state = serde_json::to_value(DslFileSource::state()).unwrap();
        state["file"]["value"] = path.display().to_string().into();
        crate::nodes::test_support::assert_node_registration_contract_with_state(
            "org.logicconduit.graph-node.dsl-file-source/v1",
            Some(state),
        );
    }
}
