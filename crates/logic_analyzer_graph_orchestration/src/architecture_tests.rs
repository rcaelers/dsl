#[test]
fn orchestration_is_the_composition_owner_above_compiler_and_runtime() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("logic-analyzer-graph-compiler"));
    assert!(manifest.contains("logic-analyzer-graph-runtime"));

    let compiler = include_str!("../../logic_analyzer_graph_compiler/Cargo.toml");
    let runtime = include_str!("../../logic_analyzer_graph_runtime/Cargo.toml");
    assert!(!compiler.contains("logic-analyzer-graph-runtime"));
    assert!(!runtime.contains("logic-analyzer-graph-compiler"));
}
