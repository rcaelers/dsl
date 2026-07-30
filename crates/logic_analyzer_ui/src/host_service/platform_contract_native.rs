use std::path::{Path, PathBuf};

use signal_processing::PersistentStoreConfig;
use signal_processing::derived_word_store::PersistentCacheEntrySnapshot;

pub(crate) struct OpenDialog<'a> {
    pub(crate) title: &'a str,
    pub(crate) filter_label: &'a str,
    pub(crate) extensions: &'a [&'a str],
    pub(crate) initial_directory: Option<&'a Path>,
}

pub(crate) struct SaveDialog<'a> {
    pub(crate) title: &'a str,
    pub(crate) default_file_name: &'a str,
    pub(crate) filter_label: &'a str,
    pub(crate) extensions: &'a [&'a str],
    pub(crate) initial_directory: Option<&'a Path>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CacheClearStats {
    pub(crate) removed_entries: usize,
    pub(crate) removed_bytes: u64,
}

pub(crate) trait PlatformHostService {
    fn choose_open_file(&mut self, dialog: OpenDialog<'_>) -> Option<PathBuf>;

    fn choose_save_file(&mut self, dialog: SaveDialog<'_>) -> Option<PathBuf>;

    fn choose_directory(&mut self) -> Option<PathBuf>;

    fn load_graph(&mut self, path: &Path) -> Result<node_graph::GraphState, String>;

    fn save_graph(&mut self, path: &Path, graph: &serde_json::Value) -> Result<(), String>;

    fn clear_cache_entry(
        &mut self,
        config: &PersistentStoreConfig,
    ) -> Result<CacheClearStats, String>;

    fn clear_cache(&mut self, directory: &Path) -> Result<CacheClearStats, String>;

    fn inspect_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<PersistentCacheEntrySnapshot>, String>;
}
