fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_node_graph_contains_no_sigrok_host_cases() {
    let sources = [
        ("definition API", include_str!("api/node.rs")),
        ("registry", include_str!("runtime/registry.rs")),
    ];
    for (component, source) in sources {
        let source = implementation_source(source);
        for token in ["Sigrok", "sigrok", "Python decoder"] {
            assert!(
                !source.contains(token),
                "generic node-graph {component} contains decoder-host token {token:?}"
            );
        }
    }
}

#[test]
fn embedded_file_controls_use_an_injected_portable_dialog() {
    let manifest = include_str!("../Cargo.toml");
    let api = include_str!("api/control.rs");
    let builtins = include_str!("api/builtins.rs");
    let widget = include_str!("widget/graph/widget.rs");

    assert!(!manifest.contains("rfd"));
    assert!(!manifest.contains("native-file-dialog"));
    assert!(!api.contains("target_arch"));
    assert!(api.contains("pub trait FileDialogService"));
    assert!(builtins.contains("context.pick_file"));
    assert!(widget.contains("set_file_dialog_service"));
}
