#[test]
fn graph_capabilities_have_no_viewer_or_ui_dependency() {
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
fn graph_capabilities_use_the_supported_node_graph_namespace() {
    let sources = [
        include_str!("node/catalog.rs"),
        include_str!("node/contracts.rs"),
        include_str!("node_support/contracts.rs"),
    ];
    for line in sources
        .into_iter()
        .flat_map(str::lines)
        .filter(|line| line.contains("node_graph::"))
    {
        assert!(
            line.contains("node_graph::api::"),
            "graph capabilities bypasses node_graph::api: {line}"
        );
    }
}

#[test]
fn graph_capabilities_do_not_own_graph_or_payload_inventory_registration() {
    let facade = include_str!("node/mod.rs");
    assert!(!facade.contains("GraphNodeRegistration"));
    assert!(!facade.contains("PayloadRegistration"));
    assert!(!facade.contains("graph_node_registrations"));
}
