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
        ("saved subscriptions", include_str!("saved_graph.rs")),
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
fn inventory_assembly_does_not_import_the_builtin_node_module() {
    let sources = [
        ("graph compiler", include_str!("graph.rs")),
        (
            "graph-node inventory",
            include_str!("graph_node_registration.rs"),
        ),
        (
            "collected-payload inventory",
            include_str!("collected_payload_registration.rs"),
        ),
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

    let saved_graph = implementation_source(include_str!("saved_graph.rs"));
    assert!(
        !saved_graph.contains("viewer_selection"),
        "saved-payload discovery must consume the supplied plan instead of UI selection state"
    );

    let crate_facade = include_str!("lib.rs");
    assert!(
        crate_facade.contains("#[cfg(test)]\nmod viewer_selection;"),
        "the transitional manifest helper must remain test-only"
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
            && !implementation.contains("SamplingQualifier {")
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
