use std::path::{Path, PathBuf};

use logic_analyzer_ui::{CacheClearStats, CacheEntrySnapshot, HostService, OpenDialog, SaveDialog};
use signal_processing::PersistentStoreConfig;

use crate::services::PlatformServices;

pub(crate) fn standard_services() -> PlatformServices {
    PlatformServices::with_host_service(Box::new(NativeHostService))
}

struct NativeHostService;

impl HostService for NativeHostService {
    fn choose_open_file(&mut self, request: OpenDialog<'_>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(request.title)
            .add_filter(request.filter_label, request.extensions);
        if let Some(directory) = request.initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog.pick_file()
    }

    fn choose_save_file(&mut self, request: SaveDialog<'_>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(request.title)
            .set_file_name(request.default_file_name)
            .add_filter(request.filter_label, request.extensions);
        if let Some(directory) = request.initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog.save_file()
    }

    fn choose_directory(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new().pick_folder()
    }

    fn load_graph(&mut self, path: &Path) -> Result<node_graph::GraphState, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        serde_json::from_str(&json)
            .map_err(|error| format!("could not parse {}: {error}", path.display()))
    }

    fn save_graph(&mut self, path: &Path, graph: &serde_json::Value) -> Result<(), String> {
        let json = serde_json::to_string_pretty(graph)
            .map_err(|error| format!("could not serialize graph: {error}"))?;
        std::fs::write(path, json)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }

    fn clear_cache_entry(
        &mut self,
        config: &PersistentStoreConfig,
    ) -> Result<CacheClearStats, String> {
        signal_processing::clear_cache_entry(config)
            .map(|stats| CacheClearStats {
                removed_entries: stats.removed_entries,
                removed_bytes: stats.removed_bytes,
            })
            .map_err(|error| error.to_string())
    }

    fn clear_cache(&mut self, directory: &Path) -> Result<CacheClearStats, String> {
        signal_processing::clear_cache(directory)
            .map(|stats| CacheClearStats {
                removed_entries: stats.removed_entries,
                removed_bytes: stats.removed_bytes,
            })
            .map_err(|error| error.to_string())
    }

    fn inspect_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<CacheEntrySnapshot>, String> {
        signal_processing::derived_word_store::inspect_cache_entry(config)
            .map(|entry| {
                entry.map(|entry| CacheEntrySnapshot {
                    total_bytes: entry.total_bytes,
                    data_bytes: entry.data_bytes,
                    index_bytes: entry.index_bytes,
                    item_count: entry.word_count,
                    index_item_count: entry.block_count as u64,
                    first_timestamp_ns: entry.first_timestamp_ns,
                    last_timestamp_ns: entry.last_timestamp_ns,
                })
            })
            .map_err(|error| error.to_string())
    }
}
