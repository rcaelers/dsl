#[test]
fn generic_registry_contains_no_builtin_payload_or_protocol_identity() {
    for source in [
        include_str!("lib.rs"),
        include_str!("payload_registration.rs"),
        include_str!("registry.rs"),
    ] {
        for forbidden in [
            "ProtocolPacket",
            "ProtocolValue",
            "org.logicconduit.trigger",
            "org.logicconduit.protocol-packet",
        ] {
            assert!(
                !source.contains(forbidden),
                "generic graph registry contains concrete payload token {forbidden:?}"
            );
        }
    }
}
