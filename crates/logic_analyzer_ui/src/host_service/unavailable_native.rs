use std::path::{Path, PathBuf};

use signal_processing::PersistentStoreConfig;
use signal_processing::derived_word_store::PersistentCacheEntrySnapshot;

use super::contract::HostService;
use super::platform_contract::{CacheClearStats, OpenDialog, PlatformHostService, SaveDialog};

struct UnavailableNativeHostService;

impl PlatformHostService for UnavailableNativeHostService {
    fn choose_open_file(&mut self, request: OpenDialog<'_>) -> Option<PathBuf> {
        let _ = (
            request.title,
            request.filter_label,
            request.extensions,
            request.initial_directory,
        );
        None
    }

    fn choose_save_file(&mut self, request: SaveDialog<'_>) -> Option<PathBuf> {
        let _ = (
            request.title,
            request.default_file_name,
            request.filter_label,
            request.extensions,
            request.initial_directory,
        );
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
    ) -> Result<Option<PersistentCacheEntrySnapshot>, String> {
        Err(unavailable())
    }
}

impl HostService for UnavailableNativeHostService {}

fn unavailable() -> String {
    "native host integration is not enabled".into()
}

pub(crate) fn standard_host_service() -> Box<dyn HostService> {
    Box::new(UnavailableNativeHostService)
}
