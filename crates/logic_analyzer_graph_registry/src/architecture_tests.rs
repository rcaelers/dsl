#[test]
fn registry_depends_only_on_contract_and_lower_level_crates() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "logic-analyzer-graph-compiler",
        "logic-analyzer-graph-runtime",
        "logic-analyzer-graph-nodes",
        "logic-analyzer-ui",
        "logic-analyzer-platform",
        "egui",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "graph registry manifest contains {forbidden}"
        );
    }
}

#[test]
fn registry_facade_has_no_crate_internal_redirects() {
    let facade = include_str!("lib.rs");
    assert!(!facade.contains("pub(crate) use"));
}
