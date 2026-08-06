use super::contract::HostService;
use crate::app_services::AppServices;
use crate::node_catalog_service::NodeCatalogService;
use crate::preferences::PreferencesWindow;

#[test]
fn application_components_accept_ui_owned_host_ports() {
    let _: fn(Box<dyn HostService>) -> AppServices = AppServices::with_host_service;
    let _: fn(&mut PreferencesWindow, &egui::Context, &mut [Box<dyn NodeCatalogService>]) =
        PreferencesWindow::show;
}

#[test]
fn ui_host_ports_have_no_cache_path_or_command_transport_details() {
    // Cargo metadata and workspace scripts enforce crate and target boundaries, but cannot detect
    // unrelated type vocabulary or intra-crate transport state, so these remain source assertions.
    let contract = include_str!("contract.rs");
    for detail in ["DecodedBlockCache", "clear_cache", "inspect_cache_entry"] {
        assert!(
            !contract.contains(detail),
            "the host-service contract contains cache detail {detail:?}"
        );
    }

    let preferences = include_str!("../preferences/implementation.rs");
    assert!(
        !preferences.contains("PathBuf"),
        "preferences must delegate directory ownership to NodeCatalogService"
    );

    let state = include_str!("../app_platform/state.rs");
    for detail in [
        "crossbeam_channel",
        "NATIVE_MENU_BRIDGE",
        "native_menu_commands",
    ] {
        assert!(
            !state.contains(detail),
            "UI application state contains host command transport detail {detail:?}"
        );
    }
}
