//! Portable application interaction and panel composition.
//!
//! `logic_analyzer_ui` composes widgets and application services through explicit
//! graph, host, capture, and export ports. It owns no concrete graph-node
//! definitions, processing execution policy, or target selection; platform and
//! application roots supply those capabilities through the corresponding contracts.

mod about;
mod app;
#[cfg(test)]
mod app_architecture_tests;
mod app_platform;
mod app_services;
mod application_settings;
mod capture_analysis_lifecycle;
mod capture_export_service;
mod collected_output_presentation;
mod decoder_panel;
mod decoder_table_presentation;
mod graph_run_lifecycle;
mod graph_service;
mod headless;
mod host_service;
mod live_capture;
mod memory_panel;
mod node_catalog_service;
mod node_registry;
mod output_downloads;
mod panel_presentation;
mod plugin_panel;
mod preferences;
mod presentation_catalogs;
mod product;
mod sampling_overlay_presentation;
mod symbol_fonts;
#[cfg(test)]
mod test_contracts_tests;
mod timeline_marker_bindings;
mod toast;
mod viewer_selection;

pub use app::{App, DemoGraph};
pub use app_services::AppServices;
pub use application_settings::{ApplicationSettings, default_input_bindings};
pub use capture_export_service::{
    CaptureExportCompletion, CaptureExportDescriptor, CaptureExportFormat, CaptureExportService,
    CaptureExportStatus, unavailable_capture_export_service,
};
pub use headless::{
    HeadlessCacheReport, HeadlessGraphRunner, HeadlessNodeProgress, HeadlessRunError,
    HeadlessRunEvent, HeadlessRunReport,
};
pub use host_service::{
    DownloadableOutput, HostCommand, HostService, HostUiCapabilities, ModifierKeyLabels,
    OpenDialog, SaveDialog,
};
pub use node_catalog_service::{NodeCatalogService, NodeCatalogSnapshot};
pub use node_registry::build_node_registry;
pub use panel_presentation::{
    ApplicationPanelIcon, DECODER_PANEL_ICON, LOG_PANEL_ICON, LOGIC_ANALYZER_PANEL_ICON,
    MEMORY_PANEL_ICON, NODE_GRAPH_PANEL_ICON, TRIGGERS_PANEL_ICON, WATCHES_PANEL_ICON,
};
pub use plugin_panel::{PluginPanel, PluginPanelContext, PluginPanelIcon, UiPanelRegistration};
pub use product::{APPLICATION_ID, APPLICATION_NAME};
