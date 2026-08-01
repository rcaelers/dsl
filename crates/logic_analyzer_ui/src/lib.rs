mod about;
mod app;
mod app_platform;
mod app_services;
mod application_settings;
mod capture_export_service;
mod collected_output_presentation;
mod decoder_panel;
mod decoder_table_presentation;
mod graph_service;
mod host_service;
mod live_capture;
mod memory_panel;
mod node_registry;
mod plugin_panel;
mod preferences;
mod product;
mod sampling_overlay_presentation;
mod symbol_fonts;
#[cfg(test)]
mod test_contracts_tests;
mod toast;
mod viewer_selection;

pub use app::{App, DemoGraph};
pub use app_services::{AppServices, ApplicationStoragePaths};
pub use application_settings::{ApplicationSettings, default_input_bindings};
pub use host_service::{
    CacheClearStats, CacheEntrySnapshot, DecodedBlockCacheSnapshot, HostCommand, HostService,
    OpenDialog, SaveDialog,
};
pub use node_registry::build_node_registry;
pub use plugin_panel::{PluginPanel, PluginPanelContext, PluginPanelIcon, UiPanelRegistration};
pub use product::{APPLICATION_ID, APPLICATION_NAME};
