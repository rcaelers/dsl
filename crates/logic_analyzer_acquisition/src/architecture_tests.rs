#[test]
fn acquisition_contracts_do_not_select_devices_sessions_graphs_or_ui() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("acquisition manifest is readable");
    for forbidden in [
        "signal-capture-session",
        "logic-analyzer-device",
        "logic-analyzer-graph",
        "logic-analyzer-ui",
        "egui",
        "platform =",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "device-neutral acquisition depends on higher-level owner {forbidden:?}"
        );
    }

    let sources = [
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/driver.rs"))
            .expect("driver source is readable"),
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/trigger.rs"))
            .expect("trigger source is readable"),
    ];
    for forbidden in ["DSLogic", "U3Pro16", "u3pro16"] {
        assert!(
            sources.iter().all(|source| !source.contains(forbidden)),
            "device-neutral acquisition contains concrete device token {forbidden:?}"
        );
    }
}
