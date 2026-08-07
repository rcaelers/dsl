#[test]
fn trigger_domain_depends_only_on_neutral_capture_values() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("trigger manifest is readable");
    for forbidden in [
        "signal-capture-session",
        "logic-analyzer-acquisition",
        "logic-analyzer-device",
        "logic-analyzer-graph",
        "logic-analyzer-ui",
        "trigger-editor",
        "logic-analyzer-viewer",
        "egui",
        "platform =",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "trigger domain depends on higher-level owner {forbidden:?}"
        );
    }
}
