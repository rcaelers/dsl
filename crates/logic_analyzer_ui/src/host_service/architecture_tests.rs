#[test]
fn application_orchestration_depends_on_the_ui_owned_host_service() {
    let application = include_str!("../app.rs");
    assert!(application.contains("Box<dyn HostService>"));
    assert!(application.contains("AppServices"));

    for (name, source) in [
        (
            "native application hooks",
            include_str!("../app_platform/native_hooks.rs"),
        ),
        (
            "native preferences",
            include_str!("../preferences/native.rs"),
        ),
    ] {
        for direct_effect in [
            "rfd::",
            "load_from_path",
            "save_to_path",
            "signal_processing::clear_cache",
        ] {
            assert!(
                !source.contains(direct_effect),
                "{name} must use HostService rather than {direct_effect}"
            );
        }
    }
}
