mod about;
mod app;
mod app_platform;
mod application_config;
mod capture_export_service;
mod collected_output_presentation;
mod decoder_panel;
mod decoder_table_presentation;
mod graph_service;
mod host_service;
mod input_binding_config;
mod live_capture;
mod node_registry;
mod plugin_panel;
mod preferences;
mod product;
mod sampling_overlay_presentation;
#[cfg(test)]
mod test_contracts_tests;
mod toast;
mod viewer_selection;

use std::sync::OnceLock;

pub use app::App;
#[cfg(target_os = "macos")]
pub use app_platform::{
    NativeMenuCommand, dispatch_native_menu_command, set_recent_files_listener,
};
use input_bindings::InputBindings;
pub use node_registry::build_node_registry;
pub use plugin_panel::{PluginPanel, PluginPanelContext, PluginPanelIcon, UiPanelRegistration};
pub use product::{APPLICATION_ID, APPLICATION_NAME};

pub fn application_input_bindings() -> &'static InputBindings {
    static BINDINGS: OnceLock<InputBindings> = OnceLock::new();
    BINDINGS.get_or_init(input_binding_config::load)
}
