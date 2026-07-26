fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_trigger_editor_contains_no_provider_or_protocol_cases() {
    let source = implementation_source(include_str!("lib.rs"));
    for token in [
        "U3Pro16",
        "DSLogic",
        "SPI",
        "UART",
        "Binary Decoder",
        "demo:",
    ] {
        assert!(
            !source.contains(token),
            "generic trigger editor contains concrete token {token:?}"
        );
    }
}
