fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn ui_manifest_has_no_concrete_test_composition_dependencies() {
    let manifest = include_str!("../../Cargo.toml");
    for dependency in [
        "logic-analyzer-capture-export",
        "logic-analyzer-graph-nodes",
        "logic-analyzer-test-support",
    ] {
        assert!(
            !manifest.contains(dependency),
            "UI tests must use UI-owned service, catalog, and acquisition fakes; {dependency} composition belongs outside the UI crate"
        );
    }
}

#[test]
fn generic_ui_capture_components_contain_no_provider_model_or_sigrok_contracts() {
    let sources = [
        ("application", include_str!("../app.rs")),
        ("coordinator contract", include_str!("implementation.rs")),
        ("native coordinator", include_str!("native.rs")),
    ];
    let forbidden = [
        "DeterministicFake",
        "BufferedFake",
        "U3Pro16",
        "u3pro16",
        "Sigrok",
        "sigrok",
        "Python decoder",
    ];

    for (component, source) in sources {
        let source = implementation_source(source);
        for token in forbidden {
            assert!(
                !source.contains(token),
                "generic UI {component} source contains concrete token {token:?}"
            );
        }
    }
}

#[test]
fn native_capture_session_storage_is_supplied_by_the_host() {
    let source = include_str!("native.rs");

    assert!(
        !source.contains("app_platform"),
        "native capture must receive its session directory through the UI service contract"
    );
    assert!(
        source.contains("capture_session_directory: Option<PathBuf>"),
        "native capture configuration must keep the host-provided session directory explicit"
    );
}
