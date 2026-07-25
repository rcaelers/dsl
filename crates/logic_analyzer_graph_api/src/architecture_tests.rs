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
