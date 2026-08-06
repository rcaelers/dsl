#[test]
fn graph_capabilities_have_no_viewer_or_ui_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "logic-analyzer-graph-compiler",
        "logic-analyzer-graph-registry",
        "logic-analyzer-graph-nodes",
        "logic-analyzer-graph-runtime",
        "logic-analyzer-ui",
        "logic-analyzer-viewer",
        "egui",
    ] {
        assert!(!manifest.contains(forbidden));
    }

    let contracts = include_str!("node_support/contracts.rs");
    assert!(!contracts.contains("logic_analyzer_viewer"));
    assert!(!contracts.contains("ViewerLaneRenderer"));
    assert!(!contracts.contains("egui::"));
    assert!(contracts.contains("renderer_key"));
}

#[test]
fn graph_capabilities_use_the_neutral_document_contract() {
    let sources = [
        include_str!("node/contracts.rs"),
        include_str!("node_support/contracts.rs"),
    ];
    assert!(
        sources
            .iter()
            .any(|source| source.contains("node_graph_document::"))
    );
    assert!(
        sources
            .iter()
            .all(|source| !source.contains("node_graph::"))
    );
}

#[test]
fn graph_capabilities_do_not_own_graph_or_payload_inventory_registration() {
    let facade = include_str!("node/mod.rs");
    assert!(!facade.contains("GraphNodeRegistration"));
    assert!(!facade.contains("PayloadRegistration"));
    assert!(!facade.contains("graph_node_registrations"));
    assert!(!facade.contains("ProtocolPacketPresentationRegistration"));
    assert!(!facade.contains("protocol_packet_display"));
    assert!(!facade.contains("DirectoryNodeCatalog"));
    assert!(!facade.contains("PathBuf"));
}
