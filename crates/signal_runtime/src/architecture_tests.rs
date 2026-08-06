fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn runtime_has_no_concrete_signal_domain_special_cases() {
    // Cargo metadata proves that concrete signal owners are not dependencies, but it cannot detect
    // dispatch on their type names, so this remains an intentional source-level assertion.
    let sources = [
        (
            "application-manager contract",
            include_str!("app_manager/contract.rs"),
        ),
        (
            "cooperative application manager",
            include_str!("app_manager/cooperative.rs"),
        ),
        (
            "application manager",
            include_str!("app_manager/implementation.rs"),
        ),
        (
            "cooperative manager",
            include_str!("cooperative_manager.rs"),
        ),
        ("errors", include_str!("errors.rs")),
        ("graph", include_str!("graph.rs")),
        ("manager", include_str!("manager.rs")),
        ("node", include_str!("node.rs")),
        ("pipeline", include_str!("pipeline.rs")),
        ("ports", include_str!("ports.rs")),
        ("protocol", include_str!("protocol.rs")),
        ("receiver", include_str!("receiver.rs")),
        ("scheduler", include_str!("scheduler.rs")),
        ("sender", include_str!("sender.rs")),
        ("type registry", include_str!("type_registry.rs")),
        ("watchdog", include_str!("watchdog.rs")),
    ];

    for (component, source) in sources {
        let source = implementation_source(source);
        for forbidden in [
            "CaptureSession",
            "DerivedLanes",
            "EdgeQuery",
            "NumberSample",
            "SampleBlock",
            "TextSample",
        ] {
            assert!(
                !source.contains(forbidden),
                "generic runtime component {component} contains domain token {forbidden:?}"
            );
        }
    }
}
