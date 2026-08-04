#[test]
fn orchestration_is_the_composition_owner_above_compiler_and_runtime() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("logic-analyzer-graph-compiler"));
    assert!(manifest.contains("logic-analyzer-graph-runtime"));
}
