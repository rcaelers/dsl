#[test]
fn application_orchestration_depends_on_the_ui_owned_host_service() {
    let application = include_str!("../app.rs");
    assert!(application.contains("Box<dyn HostService>"));
    assert!(application.contains("AppServices"));

    for (name, source) in [
        (
            "application hooks",
            include_str!("../app_platform/hooks.rs"),
        ),
        (
            "preferences",
            include_str!("../preferences/implementation.rs"),
        ),
    ] {
        for direct_effect in ["rfd::", "load_from_path", "save_to_path"] {
            assert!(
                !source.contains(direct_effect),
                "{name} must use HostService rather than {direct_effect}"
            );
        }
    }

    let contract = include_str!("contract.rs");
    assert!(!contract.contains("DecodedBlockCache"));
    assert!(!contract.contains("clear_cache"));
    assert!(!contract.contains("inspect_cache_entry"));
}

#[test]
fn preferences_use_the_ui_owned_catalog_service_on_every_target() {
    let module = include_str!("../preferences/mod.rs");
    let implementation = include_str!("../preferences/implementation.rs");

    assert!(!module.contains("target_arch"));
    assert!(implementation.contains("&mut dyn NodeCatalogService"));
    assert!(implementation.contains("catalog.add_directory()"));
    assert!(!implementation.contains("PathBuf"));
}

#[test]
fn host_command_transport_is_not_stored_in_ui_state() {
    let state = include_str!("../app_platform/state.rs");
    let hooks = include_str!("../app_platform/hooks.rs");

    for transport_detail in [
        "crossbeam_channel",
        "NATIVE_MENU_BRIDGE",
        "native_menu_commands",
    ] {
        assert!(
            !state.contains(transport_detail),
            "UI application state must not own host command transport detail {transport_detail:?}"
        );
    }
    assert!(hooks.contains("host_service.take_commands()"));
}
