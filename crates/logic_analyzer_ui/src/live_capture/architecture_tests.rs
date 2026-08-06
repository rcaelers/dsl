fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn generic_ui_capture_components_have_no_device_or_protocol_special_cases() {
    // Cargo metadata rejects concrete implementation dependencies, but it cannot detect branching
    // on device/protocol names or intra-crate platform reach-through, so this remains an intentional
    // source-level assertion.
    let coordinator = implementation_source(include_str!("coordinator.rs"));
    assert!(
        !coordinator.contains("app_platform"),
        "capture storage must use injected services instead of application platform state"
    );

    let sources = [
        ("application", include_str!("../app.rs")),
        ("coordinator contract", include_str!("implementation.rs")),
        ("capture coordinator", include_str!("coordinator.rs")),
    ];
    let forbidden = [
        "Binary Decoder",
        "I2C",
        "Python decoder",
        "SPI",
        "Sigrok",
        "UART",
        "U3Pro16",
        "u3pro16",
        "sigrok",
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
