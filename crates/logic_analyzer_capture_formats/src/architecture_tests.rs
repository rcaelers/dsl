#[test]
fn format_support_is_private_and_sources_use_one_portable_module_tree() {
    let crate_root = include_str!("lib.rs");
    assert!(
        !crate_root
            .lines()
            .any(|line| line.trim() == "pub mod support;")
    );

    for module in [
        include_str!("dsl_file/mod.rs"),
        include_str!("sigrok_file/mod.rs"),
    ] {
        assert!(!module.contains("target_arch"));
        assert!(module.contains("mod implementation;"));
        assert!(module.contains("mod path_compatibility;"));
    }
}

#[test]
fn source_factories_receive_neutral_host_capabilities() {
    for facade in [
        include_str!("dsl_file/facade.rs"),
        include_str!("sigrok_file/facade.rs"),
    ] {
        assert!(facade.contains("Factory: Send + Sync"));
        assert!(facade.contains("ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>"));
        assert!(facade.contains("fn metadata("));
        assert!(!facade.contains("Box<dyn ProcessNode>"));
    }
}
