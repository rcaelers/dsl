fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_collection_compiler_has_no_builtin_payload_or_protocol_checks() {
    let sources = [
        ("graph lowering", include_str!("graph.rs")),
        ("data collector", include_str!("data_collector.rs")),
    ];
    let forbidden = [
        "CollectedDataKind",
        "CollectedValueKind",
        "DerivedLaneData",
        "org.logicconduit.digital-sample",
        "org.logicconduit.word",
        "org.logicconduit.trigger",
        "org.logicconduit.number-sample",
        "org.logicconduit.text-sample",
        "\"SPI Decoder\"",
        "\"Binary Decoder\"",
        "\"UART Decoder\"",
        "\"Bits\"",
        "\"Data\"",
    ];

    for (component, source) in sources {
        let source = implementation_source(source);
        for token in forbidden {
            assert!(
                !source.contains(token),
                "generic compiler {component} contains built-in payload or protocol token {token:?}"
            );
        }
    }
}

#[test]
fn generic_compiler_contains_no_concrete_capture_provider_or_decoder_host() {
    let source = implementation_source(include_str!("graph.rs"));
    for token in [
        "DeterministicFake",
        "BufferedFake",
        "U3Pro16",
        "u3pro16",
        "Sigrok",
        "sigrok",
        "Python decoder",
    ] {
        assert!(
            !source.contains(token),
            "generic compiler contains concrete provider or decoder-host token {token:?}"
        );
    }
}

#[test]
fn registry_consumption_does_not_import_the_builtin_node_module() {
    let sources = [("graph compiler", include_str!("graph.rs"))];

    for (component, source) in sources {
        let implementation = implementation_source(source);
        assert!(
            !implementation.contains("crate::nodes")
                && !implementation.contains("logic_analyzer_graph_nodes"),
            "{component} must consume registry contracts without importing built-in nodes"
        );
    }
}

#[test]
fn compiler_manifest_has_no_concrete_graph_node_dependency() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        !manifest.contains("logic-analyzer-graph-nodes"),
        "compiler tests must use local runtime-builder contracts; built-in-node composition belongs in the workspace integration package"
    );
    assert!(manifest.contains("logic-analyzer-graph-registry"));
    assert!(manifest.contains("logic-analyzer-graph-plan"));
    assert!(!manifest.contains("logic-analyzer-graph-runtime"));
    assert!(
        !manifest.contains("inventory ="),
        "compiler must not collect plugin inventory directly"
    );
}

#[test]
fn compiler_does_not_construct_editor_registries() {
    let sources = [("compiler facade", include_str!("graph_lowerer.rs"))];

    for (component, source) in sources {
        let implementation = implementation_source(source);
        assert!(
            !implementation.contains("NodeTypeRegistry")
                && !implementation.contains("build_node_registry")
                && !implementation.contains("apply_node"),
            "{component} must not construct the node editor registry"
        );
    }
}

#[test]
fn compiler_facade_exposes_no_viewer_selection_controls() {
    let facade = implementation_source(include_str!("graph_lowerer.rs"));
    for token in [
        "synchronize_viewer_selections",
        "viewer_output_selections",
        "set_viewer_output_selected",
        "ViewerOutputSelection",
    ] {
        assert!(
            !facade.contains(token),
            "compiler facade must not expose UI viewer-selection control {token}"
        );
    }
}

#[test]
fn production_compiler_consumes_application_supplied_output_subscriptions() {
    let facade = implementation_source(include_str!("graph_lowerer.rs"));
    assert!(
        facade.contains("OutputSubscriptionPlan"),
        "compiler facade must accept the application-owned subscription plan"
    );

    let crate_facade = include_str!("lib.rs");
    assert!(
        !crate_facade.contains("saved_graph") && !crate_facade.contains("viewer_selection"),
        "saved Viewer compatibility must not remain in the compiler crate"
    );
}

#[test]
fn compiler_synthesizes_only_application_neutral_collectors() {
    let graph = include_str!("graph.rs");
    let collectors = graph
        .split_once("fn with_output_collectors")
        .expect("collector lowering function")
        .1
        .split_once("pub(crate) fn lower_with_subscriptions")
        .expect("lowering function follows collector construction")
        .0;
    assert!(collectors.contains("OUTPUT_SUBSCRIPTION_BUILDER_NAME"));
    assert!(
        !collectors.contains("\"Viewer\"") && !collectors.contains("AUTO_VIEW"),
        "compiler-generated collection must not construct or identify a concrete Viewer node"
    );
}

#[test]
fn lowering_consumes_graph_semantics_instead_of_runtime_builders() {
    let graph = implementation_source(include_str!("graph.rs"));
    assert!(!graph.contains("RuntimeBuilder"));
    let lowering = graph
        .split_once("fn with_output_collectors")
        .expect("output collector lowering")
        .1;
    assert!(lowering.contains("registry.semantics("));
}

#[test]
fn capture_discovery_consumes_explicit_capture_capabilities() {
    let graph = include_str!("graph.rs");

    assert!(graph.contains("builders.capture_source("));
    assert!(graph.contains("builders.live_capture("));
    assert!(!graph.contains("builder.capture_presentation("));
    assert!(!graph.contains("builder.capture_cache_identity("));
    assert!(!graph.contains("builder.live_capture_feature("));
    assert!(!graph.contains("builder.trigger_configuration("));
    assert!(!graph.contains("builder.apply_live_capture_edit("));
}

#[test]
fn presentation_and_timeline_consumers_use_their_explicit_capabilities() {
    let graph = include_str!("graph.rs");

    assert!(graph.contains("registry.presentation("));
    assert!(graph.contains("builders.timeline("));
    assert!(!graph.contains("registry.get("));
    assert!(!graph.contains(".get(node.def_name())"));
    assert!(!graph.contains("builders.get("));
    assert!(!graph.contains("builder.timeline_markers("));
}

#[test]
fn lowerer_accepts_host_capability_bundles_at_composition() {
    let lowerer = include_str!("graph_lowerer.rs");

    assert!(lowerer.contains("GraphNodeCapabilityOverride"));
    assert!(lowerer.contains("with_capability_overrides"));
    assert!(lowerer.contains("with_capability_overrides_and_infrastructure"));
}

#[test]
fn compiler_returns_neutral_sampling_and_table_plans() {
    let implementation = implementation_source(include_str!("graph.rs"));
    assert!(
        !implementation.contains("DecoderTableRegistry")
            && !implementation.contains("overlay: SamplingOverlay"),
        "compiler runtime context must expose resolved plans instead of constructing UI registries"
    );
}

#[test]
fn compiler_has_no_production_ui_dependencies() {
    let manifest = include_str!("../Cargo.toml");
    let production_dependencies = manifest
        .split_once("[dev-dependencies]")
        .map_or(manifest, |(production, _)| production);
    assert!(!production_dependencies.contains("logic-analyzer-viewer"));
    assert!(!production_dependencies.contains("egui"));

    let implementation = implementation_source(include_str!("graph.rs"));
    assert!(!implementation.contains("logic_analyzer_viewer"));
    assert!(!implementation.contains("egui::"));
}

#[test]
fn compiler_facade_contains_only_compilation_owners() {
    let facade = include_str!("lib.rs");
    assert!(
        !facade.contains("pub(crate) use"),
        "crate-internal consumers must import owning modules directly instead of using the crate facade"
    );
    for forbidden in [
        "graph_compiler",
        "cache_policy",
        "run_data",
        "source_preparation",
        "worker_client",
        "worker_execution",
        "GraphWorker",
        "LiveRun",
    ] {
        assert!(
            !facade.contains(forbidden),
            "compiler facade retains runtime owner {forbidden}"
        );
    }

    let lowerer = implementation_source(include_str!("graph_lowerer.rs"));
    for forbidden in [
        "AppManager",
        "ArtifactRepository",
        "WorkExecutor",
        "SourcePreparation",
        "GraphWorkerClient",
        "LiveRun",
    ] {
        assert!(
            !lowerer.contains(forbidden),
            "stateless lowerer retains {forbidden}"
        );
    }
}

#[test]
fn compiler_uses_only_the_neutral_document_contract() {
    let sources = [
        ("data collector", include_str!("data_collector.rs")),
        ("compiler facade", include_str!("graph_lowerer.rs")),
    ];

    for (component, source) in sources {
        let source = implementation_source(source);
        assert!(
            !source.contains("node_graph::"),
            "compiler {component} depends on the node editor"
        );
    }

    let graph = include_str!("graph.rs");
    assert!(graph.contains("node_graph_document::"));
    assert!(!graph.contains("node_graph::"));
}
