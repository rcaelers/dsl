#[test]
fn cache_diagnostics_use_one_ui_snapshot_path_on_every_target() {
    let snapshot = include_str!("snapshot.rs");
    let native_hooks = include_str!("../app_platform/native_hooks.rs");
    let web_hooks = include_str!("../app_platform/wasm_hooks.rs");

    assert!(!snapshot.contains("target_arch"));
    assert!(!snapshot.contains("decoded_block_cache_stats"));
    assert!(snapshot.contains("host_service.decoded_block_cache_snapshot()"));
    assert!(!native_hooks.contains("platform_memory_snapshot"));
    assert!(!web_hooks.contains("platform_memory_snapshot"));
}
