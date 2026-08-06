#[test]
fn host_metadata_dependencies_are_instance_owned_editor_overrides() {
    // No compiled Rust probe can distinguish an instance-owned factory from a process-global
    // slot, so this remains an intentional source-level architecture assertion.
    let host_configuration = include_str!("host_configuration.rs");
    assert!(!host_configuration.contains("OnceLock"));
    assert!(!host_configuration.contains("RwLock"));
    assert!(!host_configuration.contains("static "));
    assert!(!host_configuration.contains("pub fn install_"));
    assert!(host_configuration.contains("GraphNodeEditorOverride"));

    for definition in [
        include_str!("nodes/sources/file_source/definition.rs"),
        include_str!("nodes/sources/sigrok_file_source/definition.rs"),
        include_str!("nodes/decoders/sigrok_decoder/definition.rs"),
    ] {
        assert!(!definition.contains("host_configuration"));
    }
}
