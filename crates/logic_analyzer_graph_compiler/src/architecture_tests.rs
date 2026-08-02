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
fn inventory_assembly_does_not_import_the_builtin_node_module() {
    let sources = [
        ("graph compiler", include_str!("graph.rs")),
        (
            "graph-node inventory",
            include_str!("graph_node_registration.rs"),
        ),
        ("payload inventory", include_str!("payload_registration.rs")),
    ];

    for (component, source) in sources {
        let implementation = implementation_source(source);
        assert!(
            !implementation.contains("crate::nodes")
                && !implementation.contains("logic_analyzer_graph_nodes"),
            "{component} must consume inventory contracts without importing built-in nodes"
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
}

#[test]
fn compiler_does_not_construct_editor_registries() {
    let sources = [
        ("compiler facade", include_str!("graph_compiler.rs")),
        (
            "runtime inventory",
            include_str!("graph_node_registration.rs"),
        ),
    ];

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
    let facade = implementation_source(include_str!("graph_compiler.rs"));
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
    let facade = implementation_source(include_str!("graph_compiler.rs"));
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
fn compiler_tests_build_graph_documents_without_the_widget() {
    let tests = include_str!("graph.rs")
        .split_once("#[cfg(all(test, not(target_arch = \"wasm32\")))]\nmod tests")
        .expect("graph test module boundary")
        .1;
    assert!(tests.contains("GraphDocumentBuilder"));
    assert!(!tests.contains("NodeGraphWidget"));
    assert!(!tests.contains("egui::"));
}

#[test]
fn compiler_cache_policy_uses_its_storage_backend_contract() {
    let policy = implementation_source(include_str!("cache_policy.rs"));
    assert!(!policy.contains("IndexedAnnotationStore"));
    assert!(!policy.contains("signal_processing::cleanup_cache"));

    let adapter = implementation_source(include_str!("derived_cache_backend.rs"));
    assert!(adapter.contains("IndexedAnnotationStore::open_persistent"));
    assert!(adapter.contains("derived_word_store::cleanup_cache"));

    let facade = include_str!("lib.rs");
    assert!(!facade.contains("cache_platform_native"));
    assert!(!facade.contains("cache_platform_wasm"));
    assert!(!facade.contains("target_arch = \"wasm32\""));
}

#[test]
fn compiler_uses_only_the_node_graph_api_namespace() {
    let sources = [
        ("cache policy", include_str!("cache_policy.rs")),
        ("data collector", include_str!("data_collector.rs")),
        (
            "derived-cache contract",
            include_str!("derived_cache_backend.rs"),
        ),
        ("errors", include_str!("errors.rs")),
        ("compiler facade", include_str!("graph_compiler.rs")),
        ("subscriptions", include_str!("output_subscription.rs")),
        ("run data", include_str!("run_data.rs")),
    ];

    for (component, source) in sources {
        for line in implementation_source(source)
            .lines()
            .filter(|line| line.contains("node_graph::"))
        {
            assert!(
                line.contains("node_graph::api::"),
                "compiler {component} bypasses node_graph::api: {line}"
            );
        }
    }

    let graph = include_str!("graph.rs")
        .split_once("#[cfg(all(test, not(target_arch = \"wasm32\")))]\nmod tests")
        .expect("graph test module boundary")
        .0;
    for line in graph.lines().filter(|line| line.contains("node_graph::")) {
        assert!(
            line.contains("node_graph::api::"),
            "compiler graph lowering bypasses node_graph::api: {line}"
        );
    }
}

#[test]
fn run_data_contract_is_application_neutral() {
    let implementation = include_str!("run_data.rs");
    for forbidden in [
        "logic_analyzer_viewer",
        "egui::",
        "DecoderTableRegistry",
        "WaveformPresentationRegistry",
        "NodeGraphWidget",
    ] {
        assert!(
            !implementation.contains(forbidden),
            "run-data contract contains UI type {forbidden}"
        );
    }
    for required in [
        "DerivedLanes",
        "CollectedOutputSubscription",
        "CollectedTableSubscription",
        "RunDiagnosticRegistry",
        "SourceReadinessRegistry",
    ] {
        assert!(implementation.contains(required));
    }
}
