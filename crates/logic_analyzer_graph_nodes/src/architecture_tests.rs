#[test]
fn host_metadata_dependencies_are_instance_owned_editor_overrides() {
    // No compiled Rust probe can distinguish an instance-owned factory from a process-global
    // slot, so this remains an intentional source-level architecture assertion.
    let host_configuration = include_str!("host_configuration.rs");
    assert!(!host_configuration.contains("OnceLock"));
    assert!(!host_configuration.contains("RwLock"));
    assert!(!host_configuration.contains("static "));
    assert!(!host_configuration.contains("pub fn install_"));
    assert!(host_configuration.contains("GraphNodeEditorOverride"));

    for definition in [
        include_str!("nodes/sources/file_source/definition.rs"),
        include_str!("nodes/sources/sigrok_file_source/definition.rs"),
        include_str!("nodes/decoders/sigrok_decoder/definition.rs"),
    ] {
        assert!(!definition.contains("host_configuration"));
    }
}

#[test]
fn platform_sensitive_capabilities_delegate_construction_to_processing_facades() {
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
        assert!(builder.contains("impl GraphNodeSemantics"));
        assert!(builder.contains("impl RuntimeMaterializer"));
        assert!(!builder.contains("impl RuntimeBuilder"));
        assert!(builder.contains("GraphNodeCapabilityOverride::capabilities"));
        for constructor in forbidden_constructors {
            assert!(
                !builder.contains(constructor),
                "graph runtime builder constructs platform backend with `{constructor}`"
            );
        }
    }
}

#[test]
fn platform_sensitive_nodes_register_explicit_capability_sets() {
    let source_registrations = [
        include_str!("nodes/sources/file_source/registration.rs"),
        include_str!("nodes/sources/sigrok_file_source/registration.rs"),
        include_str!("nodes/sources/dslogic_u3pro16/registration.rs"),
    ];
    let sink_registrations = [
        include_str!("nodes/sinks/file_writer/registration.rs"),
        include_str!("nodes/sinks/csv_writer/registration.rs"),
        include_str!("nodes/sinks/text_file_writer/registration.rs"),
    ];

    for registration in source_registrations {
        assert!(registration.contains("GraphNodeRegistration::capable"));
        assert!(registration.contains(".with_capture_source"));
        assert!(registration.contains(".with_presentation"));
        assert!(!registration.contains("GraphNodeRegistration::runnable"));
    }
    for registration in sink_registrations {
        assert!(registration.contains("GraphNodeRegistration::capable"));
        assert!(!registration.contains("GraphNodeRegistration::runnable"));
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
        assert!(
            facade
                .lines()
                .any(|line| { matches!(line.trim(), "mod builder;" | "pub(crate) mod builder;") })
        );
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

#[test]
fn migrated_logic_gate_registers_narrow_graph_capabilities() {
    let builder = include_str!("nodes/logic/logic_gate/builder.rs");
    let registration = include_str!("nodes/logic/logic_gate/registration.rs");

    assert!(builder.contains("impl GraphNodeSemantics for LogicGateBuilder"));
    assert!(builder.contains("impl RuntimeMaterializer for LogicGateBuilder"));
    assert!(!builder.contains("impl RuntimeBuilder for LogicGateBuilder"));
    assert!(registration.contains("GraphNodeRegistration::capable"));
}

#[test]
fn migrated_logic_primitives_register_narrow_graph_capabilities() {
    let features = [
        (
            "BufferBuilder",
            include_str!("nodes/logic/buffer/builder.rs"),
            include_str!("nodes/logic/buffer/registration.rs"),
        ),
        (
            "EdgeDetectorBuilder",
            include_str!("nodes/logic/edge_detector/builder.rs"),
            include_str!("nodes/logic/edge_detector/registration.rs"),
        ),
        (
            "SrFlipFlopBuilder",
            include_str!("nodes/logic/sr_flip_flop/builder.rs"),
            include_str!("nodes/logic/sr_flip_flop/registration.rs"),
        ),
        (
            "EventGateBuilder",
            include_str!("nodes/logic/event_gate/builder.rs"),
            include_str!("nodes/logic/event_gate/registration.rs"),
        ),
        (
            "EventControlBuilder",
            include_str!("nodes/logic/event_control/builder.rs"),
            include_str!("nodes/logic/event_control/registration.rs"),
        ),
        (
            "CounterBuilder",
            include_str!("nodes/logic/counter/builder.rs"),
            include_str!("nodes/logic/counter/registration.rs"),
        ),
    ];

    for (builder_name, builder, registration) in features {
        assert!(builder.contains(&format!("impl GraphNodeSemantics for {builder_name}")));
        assert!(builder.contains(&format!("impl RuntimeMaterializer for {builder_name}")));
        assert!(!builder.contains(&format!("impl RuntimeBuilder for {builder_name}")));
        assert!(registration.contains("GraphNodeRegistration::capable"));
        assert!(!registration.contains("GraphNodeRegistration::runnable"));
    }
}

#[test]
fn migrated_value_processors_register_narrow_graph_capabilities() {
    let features = [
        (
            "FormatterBuilder",
            include_str!("nodes/logic/formatter/builder.rs"),
            include_str!("nodes/logic/formatter/registration.rs"),
            false,
        ),
        (
            "WordFieldExtractorBuilder",
            include_str!("nodes/logic/word_field_extractor/builder.rs"),
            include_str!("nodes/logic/word_field_extractor/registration.rs"),
            true,
        ),
        (
            "WordMatcherBuilder",
            include_str!("nodes/logic/word_matcher/builder.rs"),
            include_str!("nodes/logic/word_matcher/registration.rs"),
            false,
        ),
        (
            "PacketFramerBuilder",
            include_str!("nodes/logic/packet_framer/builder.rs"),
            include_str!("nodes/logic/packet_framer/registration.rs"),
            true,
        ),
    ];

    for (builder_name, builder, registration, presents) in features {
        assert!(builder.contains(&format!("impl GraphNodeSemantics for {builder_name}")));
        assert!(builder.contains(&format!("impl RuntimeMaterializer for {builder_name}")));
        assert!(!builder.contains(&format!("impl RuntimeBuilder for {builder_name}")));
        assert!(registration.contains("GraphNodeRegistration::capable"));
        assert!(!registration.contains("GraphNodeRegistration::runnable"));
        assert_eq!(registration.contains(".with_presentation"), presents);
    }
}

#[test]
fn migrated_builtin_decoders_register_explicit_presentation_capabilities() {
    let features = [
        (
            "I2cDecoderBuilder",
            include_str!("nodes/decoders/i2c_decoder/builder.rs"),
            include_str!("nodes/decoders/i2c_decoder/registration.rs"),
        ),
        (
            "UartDecoderBuilder",
            include_str!("nodes/decoders/uart_decoder/builder.rs"),
            include_str!("nodes/decoders/uart_decoder/registration.rs"),
        ),
        (
            "SpiDecoderBuilder",
            include_str!("nodes/decoders/spi_decoder/builder.rs"),
            include_str!("nodes/decoders/spi_decoder/registration.rs"),
        ),
        (
            "ParallelDecoderBuilder",
            include_str!("nodes/decoders/parallel_decoder/builder.rs"),
            include_str!("nodes/decoders/parallel_decoder/registration.rs"),
        ),
    ];

    for (builder_name, builder, registration) in features {
        assert!(builder.contains(&format!("impl GraphNodeSemantics for {builder_name}")));
        assert!(builder.contains(&format!("impl RuntimeMaterializer for {builder_name}")));
        assert!(builder.contains(&format!("impl GraphNodePresentation for {builder_name}")));
        assert!(!builder.contains(&format!("impl RuntimeBuilder for {builder_name}")));
        assert!(registration.contains("GraphNodeRegistration::capable"));
        assert!(registration.contains(".with_presentation"));
        assert!(!registration.contains("GraphNodeRegistration::runnable"));
    }
}

#[test]
fn migrated_sigrok_decoder_replaces_only_host_materialization() {
    let builder = include_str!("nodes/decoders/sigrok_decoder/builder.rs");
    let registration = include_str!("nodes/decoders/sigrok_decoder/registration.rs");

    assert!(builder.contains("impl GraphNodeSemantics for SigrokDecoderBuilder"));
    assert!(builder.contains("impl RuntimeMaterializer for SigrokDecoderBuilder"));
    assert!(!builder.contains("impl RuntimeBuilder"));
    assert!(builder.contains(".with_materializer"));
    assert!(!builder.contains(".with_semantics"));
    assert!(registration.contains("GraphNodeRegistration::capable"));
    assert!(!registration.contains("GraphNodeRegistration::runnable"));
}

#[test]
fn migrated_subscription_recorder_and_synthetic_source_use_narrow_capabilities() {
    let viewer_builder = include_str!("nodes/sinks/viewer/builder.rs");
    let viewer_registration = include_str!("nodes/sinks/viewer/graph_registration.rs");
    let recorder_builder = include_str!("nodes/sinks/tgck_recorder/builder.rs");
    let recorder_registration = include_str!("nodes/sinks/tgck_recorder/registration.rs");
    let source_builder = include_str!("nodes/sources/test_uart_source/builder.rs");
    let source_registration = include_str!("nodes/sources/test_uart_source/registration.rs");

    for (builder, registration) in [
        (viewer_builder, viewer_registration),
        (recorder_builder, recorder_registration),
        (source_builder, source_registration),
    ] {
        assert!(builder.contains("impl GraphNodeSemantics"));
        assert!(builder.contains("impl RuntimeMaterializer"));
        assert!(!builder.contains("impl RuntimeBuilder"));
        assert!(registration.contains("GraphNodeRegistration::capable"));
        assert!(!registration.contains("GraphNodeRegistration::runnable"));
    }
    assert!(source_builder.contains("impl CaptureSourceFeature"));
    assert!(source_builder.contains("impl GraphNodePresentation"));
    assert!(source_registration.contains(".with_capture_source"));
    assert!(source_registration.contains(".with_presentation"));
}

#[test]
fn migrated_test_capture_family_registers_explicit_capture_capabilities() {
    let capture_builder = include_str!("nodes/sources/test_capture_source/builder.rs");
    let live_builder = include_str!("nodes/sources/test_capture_source/live_builder.rs");
    let registration = include_str!("nodes/sources/test_capture_source/registration.rs");

    assert!(!capture_builder.contains("impl RuntimeBuilder"));
    assert!(!live_builder.contains("impl RuntimeBuilder"));
    assert!(capture_builder.contains("impl CaptureSourceFeature"));
    assert!(capture_builder.contains("impl GraphNodePresentation"));
    assert!(live_builder.contains("impl LiveCaptureFeatureProvider"));
    assert!(registration.contains(".with_capture_source"));
    assert!(registration.contains(".with_live_capture"));
    assert!(registration.contains(".with_presentation"));
}

#[test]
fn timeline_marker_metadata_uses_explicit_registration_fields() {
    let builder = include_str!("nodes/logic/timeline_marker/builder.rs");
    let registration = include_str!("nodes/logic/timeline_marker/registration.rs");

    assert!(builder.contains("impl TimelineFeature for TimelineMarkerBuilder"));
    assert!(builder.contains("impl TimelineFeature for CursorMarkerBuilder"));
    assert_eq!(builder.matches("impl GraphNodeSemantics for").count(), 5);
    assert_eq!(builder.matches("impl RuntimeMaterializer for").count(), 5);
    assert!(!builder.contains("impl RuntimeBuilder"));
    assert_eq!(
        registration
            .matches("GraphNodeRegistration::capable")
            .count(),
        5
    );
    assert!(!registration.contains("GraphNodeRegistration::runnable"));
    assert!(registration.contains(".with_timeline"));
}
