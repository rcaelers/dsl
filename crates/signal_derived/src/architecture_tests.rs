fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_derived_owner_has_no_concrete_protocol_or_session_vocabulary() {
    // The resolved dependency graph cannot detect locally introduced protocol strings or
    // session-shaped types, so this remains an intentional source-level assertion.
    let sources = [
        (
            "collector",
            include_str!("derived_data_collector/collector.rs"),
        ),
        ("catalog", include_str!("derived_data_collector/catalog.rs")),
        ("word store", include_str!("derived_word_store/store.rs")),
        ("events", include_str!("events.rs")),
        ("payloads", include_str!("payload.rs")),
        ("sampling points", include_str!("sampling_points.rs")),
    ];
    let forbidden = ["CaptureSession", "I2C", "SPI", "UART", "live_capture"];

    for (component, source) in sources {
        let source = implementation_source(source);
        for token in forbidden {
            assert!(
                !source.contains(token),
                "generic derived-data {component} contains domain token {token:?}"
            );
        }
    }
}
