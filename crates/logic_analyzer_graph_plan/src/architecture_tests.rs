#[test]
fn graph_plan_is_neutral_between_compiler_and_runtime() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "logic-analyzer-graph-compiler",
        "logic-analyzer-graph-registry",
        "logic-analyzer-graph-runtime",
        "logic-analyzer-ui",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "graph plan depends on {forbidden}"
        );
    }
}

#[test]
fn processing_nodes_retain_only_runtime_materialization_behavior() {
    let plan = include_str!("plan/types.rs");
    assert!(plan.contains("Arc<dyn RuntimeMaterializer>"));
    assert!(!plan.contains("RuntimeBuilder"));
}
