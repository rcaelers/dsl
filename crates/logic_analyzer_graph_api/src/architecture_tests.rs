#[test]
fn graph_api_has_no_viewer_or_ui_dependency() {
    let manifest = include_str!("../Cargo.toml");
    assert!(!manifest.contains("logic-analyzer-viewer"));
    assert!(!manifest.contains("egui"));

    let contracts = include_str!("node_support/contracts.rs");
    assert!(!contracts.contains("logic_analyzer_viewer"));
    assert!(!contracts.contains("ViewerLaneRenderer"));
    assert!(!contracts.contains("egui::"));
    assert!(contracts.contains("renderer_key"));
}

#[test]
fn graph_api_uses_the_supported_node_graph_namespace() {
    let sources = [
        include_str!("node/catalog.rs"),
        include_str!("node/contracts.rs"),
        include_str!("node/graph_registration.rs"),
        include_str!("node_support/contracts.rs"),
    ];
    for line in sources
        .into_iter()
        .flat_map(str::lines)
        .filter(|line| line.contains("node_graph::"))
    {
        assert!(
            line.contains("node_graph::api::"),
            "graph API bypasses node_graph::api: {line}"
        );
    }
}
