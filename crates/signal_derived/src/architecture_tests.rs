fn implementation_source(source: &'static str) -> &'static str {
    source
        .split_once("#[cfg(test)]")
        .or_else(|| source.split_once("#[cfg(all(test"))
        .map_or(source, |(implementation, _)| implementation)
}

#[test]
fn derived_owner_has_no_session_graph_ui_or_concrete_protocol_dependency() {
    let sources = [
        include_str!("derived_data_collector/collector.rs"),
        include_str!("derived_data_collector/catalog.rs"),
        include_str!("derived_word_store/store.rs"),
        include_str!("events.rs"),
        include_str!("payload.rs"),
        include_str!("sampling_points.rs"),
    ];
    for forbidden in [
        "signal_capture_session",
        "live_capture",
        "CaptureSession",
        "node_graph",
        "logic_analyzer_",
        "egui",
        "UART",
        "SPI",
        "I2C",
    ] {
        assert!(
            sources
                .iter()
                .all(|source| !implementation_source(source).contains(forbidden)),
            "derived owner contains unrelated dependency {forbidden:?}"
        );
    }
}

#[test]
fn derived_manifest_depends_only_on_lower_level_owners() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "signal-capture-session",
        "logic-analyzer-",
        "node-graph",
        "egui",
    ] {
        assert!(!manifest.contains(forbidden));
    }
}

#[test]
fn decoded_block_cache_has_no_process_global_entry_point() {
    let cache = implementation_source(include_str!("derived_word_store/cache.rs"));
    assert!(cache.contains("pub struct DecodedBlockCacheHandle"));
    for forbidden in [
        "OnceLock",
        "static CACHE",
        "shared_cache",
        "configure_decoded_block_cache",
        "decoded_block_cache_stats",
        "reset_decoded_block_cache_stats",
    ] {
        assert!(
            !cache.contains(forbidden),
            "decoded-block cache contains process-global entry point {forbidden:?}"
        );
    }
}
