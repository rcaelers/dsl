#[test]
fn cache_diagnostics_use_one_ui_snapshot_path_on_every_target() {
    let snapshot = include_str!("snapshot.rs");
    let hooks = include_str!("../app_platform/hooks.rs");

    assert!(!snapshot.contains("target_arch"));
    assert!(!snapshot.contains("decoded_block_cache_stats"));
    assert!(snapshot.contains("self.decoded_block_cache.stats()"));
    assert!(!snapshot.contains("host_service.decoded_block_cache"));
    assert!(snapshot.contains("graph_service.inspect_derived_cache_entry"));
    assert!(!snapshot.contains("host_service.inspect_cache_entry"));
    assert!(!hooks.contains("platform_memory_snapshot"));
}
