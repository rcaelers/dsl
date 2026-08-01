use std::path::{Path, PathBuf};

use signal_processing::PersistentStoreConfig;

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
}

impl AppServices {
    /// Combines the standard graph/compiler service with a host-provided UI
    /// service implementation.
    pub fn with_host_service(host_service: Box<dyn HostService>) -> Self {
        Self {
            graph_service: standard_graph_service(),
            host_service,
        }
    }

    pub(crate) fn into_parts(self) -> (Box<dyn GraphService>, Box<dyn HostService>) {
        (self.graph_service, self.host_service)
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
