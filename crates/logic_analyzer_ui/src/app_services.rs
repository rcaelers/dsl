use std::path::{Path, PathBuf};

use input_bindings::InputBindings;
use node_graph::FileDialogService;
use signal_processing::PersistentStoreConfig;

use crate::application_settings::{ApplicationSettings, default_input_bindings};
use crate::graph_service::{GraphService, standard_graph_service};
use crate::host_service::{
    CacheClearStats, CacheEntrySnapshot, HostService, OpenDialog, SaveDialog,
};

/// The UI services selected by the application composition root.
///
/// The UI owns these ports. Hosts provide their implementations without
/// exposing native or browser details to application behavior.
pub struct AppServices {
    graph_service: Box<dyn GraphService>,
    host_service: Box<dyn HostService>,
    storage_paths: ApplicationStoragePaths,
    input_bindings: InputBindings,
    application_settings: ApplicationSettings,
    host_symbol_fonts: Vec<egui::FontData>,
    node_file_dialog: Option<Box<dyn FileDialogService>>,
}

pub(crate) struct AppServiceParts {
    pub(crate) graph_service: Box<dyn GraphService>,
    pub(crate) host_service: Box<dyn HostService>,
    pub(crate) storage_paths: ApplicationStoragePaths,
    pub(crate) input_bindings: InputBindings,
    pub(crate) application_settings: ApplicationSettings,
    pub(crate) host_symbol_fonts: Vec<egui::FontData>,
    pub(crate) node_file_dialog: Option<Box<dyn FileDialogService>>,
}

impl AppServices {
    /// Combines the standard graph/compiler service with a host-provided UI
    /// service implementation.
    pub fn with_host_service(host_service: Box<dyn HostService>) -> Self {
        Self::with_host_storage_and_configuration(
            host_service,
            ApplicationStoragePaths::default(),
            default_input_bindings(),
            ApplicationSettings::default(),
            Vec::new(),
        )
    }

    /// Combines a host service with the application-owned storage locations
    /// selected by the application composition root.
    pub fn with_host_storage_and_configuration(
        host_service: Box<dyn HostService>,
        storage_paths: ApplicationStoragePaths,
        input_bindings: InputBindings,
        application_settings: ApplicationSettings,
        host_symbol_fonts: Vec<egui::FontData>,
    ) -> Self {
        Self {
            graph_service: standard_graph_service(),
            host_service,
            storage_paths,
            input_bindings,
            application_settings,
            host_symbol_fonts,
            node_file_dialog: None,
        }
    }

    /// Supplies the host capability used by file controls embedded in graph nodes.
    pub fn with_node_file_dialog(mut self, service: Box<dyn FileDialogService>) -> Self {
        self.node_file_dialog = Some(service);
        self
    }

    pub(crate) fn into_parts(self) -> AppServiceParts {
        AppServiceParts {
            graph_service: self.graph_service,
            host_service: self.host_service,
            storage_paths: self.storage_paths,
            input_bindings: self.input_bindings,
            application_settings: self.application_settings,
            host_symbol_fonts: self.host_symbol_fonts,
            node_file_dialog: self.node_file_dialog,
        }
    }
}

/// Locations allocated by the host for application-owned data.
///
/// A missing location means that the corresponding optional capability is not
/// available. UI and compiler behavior must not infer a replacement location.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplicationStoragePaths {
    derived_cache_directory: Option<PathBuf>,
    capture_session_directory: Option<PathBuf>,
}

impl ApplicationStoragePaths {
    pub fn new(derived_cache_directory: Option<PathBuf>) -> Self {
        Self {
            derived_cache_directory,
            capture_session_directory: None,
        }
    }

    pub fn with_capture_session_directory(mut self, directory: Option<PathBuf>) -> Self {
        self.capture_session_directory = directory;
        self
    }

    pub fn derived_cache_directory(&self) -> Option<&Path> {
        self.derived_cache_directory.as_deref()
    }

    pub fn capture_session_directory(&self) -> Option<&Path> {
        self.capture_session_directory.as_deref()
    }
}

pub(crate) fn unavailable_app_services() -> AppServices {
    AppServices::with_host_service(Box::new(UnavailableHostService))
}

struct UnavailableHostService;

impl HostService for UnavailableHostService {
    fn choose_open_file(&mut self, _request: OpenDialog<'_>) -> Option<PathBuf> {
        None
    }

    fn choose_save_file(&mut self, _request: SaveDialog<'_>) -> Option<PathBuf> {
        None
    }

    fn choose_directory(&mut self) -> Option<PathBuf> {
        None
    }

    fn load_graph(&mut self, _path: &Path) -> Result<node_graph::GraphState, String> {
        Err(unavailable())
    }

    fn save_graph(&mut self, _path: &Path, _graph: &serde_json::Value) -> Result<(), String> {
        Err(unavailable())
    }

    fn clear_cache_entry(
        &mut self,
        _config: &PersistentStoreConfig,
    ) -> Result<CacheClearStats, String> {
        Err(unavailable())
    }

    fn clear_cache(&mut self, _directory: &Path) -> Result<CacheClearStats, String> {
        Err(unavailable())
    }

    fn inspect_cache_entry(
        &self,
        _config: &PersistentStoreConfig,
    ) -> Result<Option<CacheEntrySnapshot>, String> {
        Err(unavailable())
    }
}

fn unavailable() -> String {
    "host integration was not supplied by the application".into()
}

#[cfg(test)]
mod app_services_tests {
    use std::path::Path;

    use super::ApplicationStoragePaths;

    #[test]
    fn storage_directories_are_explicit_optional_capabilities() {
        let unavailable = ApplicationStoragePaths::default();
        assert_eq!(unavailable.derived_cache_directory(), None);
        assert_eq!(unavailable.capture_session_directory(), None);

        let configured = ApplicationStoragePaths::new(Some("cache/derived".into()))
            .with_capture_session_directory(Some("cache/captures".into()));
        assert_eq!(
            configured.derived_cache_directory(),
            Some(Path::new("cache/derived"))
        );
        assert_eq!(
            configured.capture_session_directory(),
            Some(Path::new("cache/captures"))
        );
    }
}
