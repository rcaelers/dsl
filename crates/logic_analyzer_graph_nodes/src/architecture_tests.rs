#[test]
fn platform_sensitive_runtime_builders_delegate_construction_to_processing_facades() {
    let builders = [
        include_str!("nodes/sources/file_source/builder.rs"),
        include_str!("nodes/sources/file_source/builder_wasm.rs"),
        include_str!("nodes/sources/sigrok_file_source/builder.rs"),
        include_str!("nodes/sources/sigrok_file_source/builder_wasm.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/builder.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/builder_wasm.rs"),
        include_str!("nodes/sinks/file_writer/builder.rs"),
        include_str!("nodes/sinks/file_writer/builder_wasm.rs"),
        include_str!("nodes/sinks/csv_writer/builder.rs"),
        include_str!("nodes/sinks/csv_writer/builder_wasm.rs"),
        include_str!("nodes/sinks/text_file_writer/builder.rs"),
        include_str!("nodes/sinks/text_file_writer/builder_wasm.rs"),
    ];
    let forbidden_constructors = [
        "DslFileSource::new",
        "SigrokFileSource::new",
        "DsLogicU3Pro16Source::open_first",
        "SyntheticCaptureSource::new",
        "BinaryFileWriter::new",
        "CsvWordWriter::new",
        "TextFileWriter::new",
        "DiscardWordWriter::new",
        "DiscardTextWriter::new",
    ];

    for builder in builders {
        for constructor in forbidden_constructors {
            assert!(
                !builder.contains(constructor),
                "graph runtime builder constructs platform backend with `{constructor}`"
            );
        }
    }
}
