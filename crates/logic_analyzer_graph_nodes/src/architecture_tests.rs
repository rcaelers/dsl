#[test]
fn platform_sensitive_runtime_builders_delegate_construction_to_processing_facades() {
    let builders = [
        include_str!("nodes/sources/file_source/builder.rs"),
        include_str!("nodes/sources/sigrok_file_source/builder.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/builder.rs"),
        include_str!("nodes/sinks/file_writer/builder.rs"),
        include_str!("nodes/sinks/csv_writer/builder.rs"),
        include_str!("nodes/sinks/text_file_writer/builder.rs"),
    ];
    let forbidden_constructors = [
        "DslFileSource::new",
        "SigrokFileSource::new",
        "DsLogicU3Pro16Source::open_first",
        "SigrokDecoder::with_work_executor",
        "discover_sigrok_decoder",
        "SyntheticCaptureSource::new",
        "BinaryFileWriter::new",
        "CsvWordWriter::new",
        "TextFileWriter::new",
        "DiscardWordWriter::new",
        "DiscardTextWriter::new",
    ];

    for builder in builders {
        assert!(builder.contains("Factory"));
        assert!(builder.contains("ProcessNodeConstruction::into_process"));
        for constructor in forbidden_constructors {
            assert!(
                !builder.contains(constructor),
                "graph runtime builder constructs platform backend with `{constructor}`"
            );
        }
    }
}

#[test]
fn portable_capture_nodes_use_one_builder_on_every_target() {
    for facade in [
        include_str!("nodes/sources/file_source/mod.rs"),
        include_str!("nodes/sources/sigrok_file_source/mod.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/mod.rs"),
        include_str!("nodes/sinks/file_writer/mod.rs"),
        include_str!("nodes/sinks/csv_writer/mod.rs"),
        include_str!("nodes/sinks/text_file_writer/mod.rs"),
    ] {
        assert!(facade.lines().any(|line| line.trim() == "mod builder;"));
        assert!(!facade.contains("builder_wasm"));
        assert!(!facade.contains("path = \"builder"));
    }
}

#[test]
fn portable_sources_forward_processing_metadata_without_platform_adapters() {
    for builder in [
        include_str!("nodes/sources/file_source/builder.rs"),
        include_str!("nodes/sources/sigrok_file_source/builder.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/builder.rs"),
    ] {
        assert!(builder.contains("CaptureSourceMetadata"));
        assert!(builder.contains("source_factory.lifecycle()"));
        assert!(!builder.contains("CapturePresentationSignal"));
        assert!(!builder.contains("SourceDataLifecycle::new"));
    }

    for facade in [
        include_str!("nodes/sources/file_source/mod.rs"),
        include_str!("nodes/sources/sigrok_file_source/mod.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/mod.rs"),
    ] {
        assert!(!facade.contains("metadata_platform"));
        assert!(!facade.contains("presentation_platform"));
        assert!(!facade.contains("live_capture_wasm"));
        assert!(!facade.contains("target_arch"));
    }

    let sources = include_str!("nodes/sources/mod.rs");
    assert!(!sources.contains("synthetic_presentation"));
    assert!(!sources.contains("file_identity_cache"));
}
