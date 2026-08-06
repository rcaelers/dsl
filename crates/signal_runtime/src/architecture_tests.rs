fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn runtime_has_no_signal_domain_or_storage_dependency() {
    let sources = [
        include_str!("app_manager/contract.rs"),
        include_str!("app_manager/cooperative.rs"),
        include_str!("app_manager/implementation.rs"),
        include_str!("cooperative_manager.rs"),
        include_str!("errors.rs"),
        include_str!("graph.rs"),
        include_str!("manager.rs"),
        include_str!("node.rs"),
        include_str!("pipeline.rs"),
        include_str!("ports.rs"),
        include_str!("protocol.rs"),
        include_str!("receiver.rs"),
        include_str!("scheduler.rs"),
        include_str!("sender.rs"),
        include_str!("type_registry.rs"),
        include_str!("watchdog.rs"),
    ];

    for forbidden in [
        "signal_capture_session",
        "platform_artifacts",
        "SampleBlock",
        "NumberSample",
        "TextSample",
        "EdgeQuery",
        "CaptureSession",
        "DerivedLanes",
    ] {
        assert!(
            sources
                .iter()
                .all(|source| !implementation_source(source).contains(forbidden)),
            "generic runtime contains domain/storage token {forbidden:?}"
        );
    }
}

#[test]
fn application_manager_is_a_portable_facade() {
    let library = include_str!("lib.rs");
    let facade = include_str!("app_manager/mod.rs");
    assert!(!library.contains("type AppManager"));
    assert!(!facade.contains("target_arch"));
    assert!(facade.contains("mod contract;"));
    assert!(facade.contains("mod cooperative;"));
    assert!(facade.contains("mod implementation;"));
    assert!(facade.contains("mod pipeline;"));
    assert!(facade.contains("AppManagerBackend"));
    assert!(facade.contains("AppManagerFactory"));
}
