#[test]
fn registry_depends_only_on_contract_and_lower_level_crates() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "logic-analyzer-graph-compiler",
        "logic-analyzer-graph-runtime",
        "logic-analyzer-graph-nodes",
        "logic-analyzer-ui",
        "node-graph =",
        "platform",
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

#[test]
fn registry_validates_explicit_capability_bundles() {
    let registration = include_str!("graph_registration.rs");
    let registry = include_str!("registry.rs");

    assert!(registration.contains("create_semantics"));
    assert!(registration.contains("create_materializer"));
    assert!(registration.contains("create_capture_source"));
    assert!(registration.contains("create_live_capture"));
    assert!(registration.contains("create_presentation"));
    assert!(registration.contains("create_timeline"));
    assert!(registry.contains("GraphNodeCapabilityBundle"));
    assert!(registry.contains("validate_capability_combinations"));
    assert!(registry.contains("duplicate host graph-capability override"));
    assert!(registry.contains("contains no replacements"));
    assert!(!registration.contains("RuntimeBuilder"));
    assert!(!registration.contains("create_builder"));
    assert!(!registry.contains("BuilderSemantics"));
    assert!(!registry.contains("compatibility_builder"));
}

#[test]
fn registry_owns_protocol_packet_presentation_inventory() {
    let registration = include_str!("protocol_packet_presentation.rs");
    let facade = include_str!("lib.rs");

    assert!(registration.contains("inventory::collect!(ProtocolPacketPresentationRegistration)"));
    assert!(registration.contains("pub fn protocol_packet_display"));
    assert!(facade.contains("ProtocolPacketPresentationRegistration"));
    assert!(facade.contains("protocol_packet_display"));
}
