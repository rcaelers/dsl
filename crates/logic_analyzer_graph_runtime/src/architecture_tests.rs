fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn runtime_has_no_ui_target_or_concrete_node_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "logic-analyzer-graph-nodes",
        "logic-analyzer-graph-compiler",
        "logic-analyzer-graph-registry",
        "logic-analyzer-capture-formats",
        "logic-analyzer-device-dslogic",
        "logic-analyzer-protocol-decoders",
        "signal-generators",
        "signal-sinks",
        "signal-transforms",
        "logic-analyzer-ui",
        "logic-analyzer-viewer",
        "egui",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "runtime manifest contains {forbidden}"
        );
    }
    assert!(manifest.contains("logic-analyzer-graph-plan"));

    for source in [
        include_str!("runtime/execution.rs"),
        include_str!("runtime/service.rs"),
        include_str!("runtime/source_preparation.rs"),
    ] {
        let source = implementation_source(source);
        for forbidden in [
            "target_arch",
            "PathBuf",
            "egui::",
            "logic_analyzer_graph_nodes",
        ] {
            assert!(
                !source.contains(forbidden),
                "runtime implementation contains {forbidden}"
            );
        }
    }
}

#[test]
fn execution_entry_points_consume_compiled_plans() {
    let execution = implementation_source(include_str!("runtime/execution.rs"));
    assert!(!execution.contains("GraphState"));
    assert!(!execution.contains("lower_with_subscriptions"));

    let service = implementation_source(include_str!("runtime/service.rs"));
    for entry in [
        "pub fn start(",
        "pub fn start_live_analysis(",
        "pub fn apply(",
    ] {
        let signature = service
            .split_once(entry)
            .unwrap_or_else(|| panic!("missing runtime entry {entry}"))
            .1
            .split_once(')')
            .expect("runtime entry signature")
            .0;
        assert!(
            signature.contains("ProcessingGraph"),
            "{entry} must consume a compiled plan"
        );
    }
}

#[test]
fn runtime_owns_processing_graph_execution_lifetimes() {
    let facade = include_str!("runtime/mod.rs");
    for owner in [
        "cache_policy",
        "execution",
        "run_data",
        "source_preparation",
    ] {
        assert!(
            facade.contains(&format!("mod {owner};")),
            "runtime does not own {owner}"
        );
    }
}

#[test]
fn runtime_never_accepts_editor_documents_or_compiler_services() {
    let sources = [
        include_str!("runtime/service.rs"),
        include_str!("runtime/execution.rs"),
    ];
    for source in sources {
        assert!(!source.contains("GraphState"));
        assert!(!source.contains("GraphLowerer"));
        assert!(!source.contains("GraphRegistry"));
    }
}

#[test]
fn runtime_does_not_query_compiler_semantics_through_materializers() {
    let execution = implementation_source(include_str!("runtime/execution.rs"));
    for forbidden in [
        "materializer.is_source",
        "materializer.is_sink",
        "materializer.is_data_",
        "materializer.source_data_lifecycle",
        "materializer.execution_state",
        "materializer.collected_lane_names",
        "materializer.sampling_overlay",
        "materializer.capture_",
    ] {
        assert!(
            !execution.contains(forbidden),
            "runtime queries compiler semantics through {forbidden}"
        );
    }
}
