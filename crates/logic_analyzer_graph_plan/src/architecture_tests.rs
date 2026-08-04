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
