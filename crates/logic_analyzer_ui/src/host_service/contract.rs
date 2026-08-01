use std::path::{Path, PathBuf};

use signal_processing::PersistentStoreConfig;

/// A request to select one existing file.
pub struct OpenDialog<'a> {
    pub title: &'a str,
    pub filter_label: &'a str,
    pub extensions: &'a [&'a str],
    pub initial_directory: Option<&'a Path>,
}

/// A request to select a destination file.
pub struct SaveDialog<'a> {
    pub title: &'a str,
    pub default_file_name: &'a str,
    pub filter_label: &'a str,
    pub extensions: &'a [&'a str],
    pub initial_directory: Option<&'a Path>,
}

/// The result of removing cached derived data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheClearStats {
    pub removed_entries: usize,
    pub removed_bytes: u64,
}

/// Host-supplied diagnostics for one cached derived-data entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheEntrySnapshot {
    pub total_bytes: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub item_count: u64,
    pub index_item_count: u64,
    pub first_timestamp_ns: Option<u64>,
    pub last_timestamp_ns: Option<u64>,
}

/// A portable application command emitted by an optional host shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostCommand {
    About,
    Preferences,
    New,
    Load,
    LoadPath(PathBuf),
    ClearRecent,
    Save,
    SaveAs,
    SaveCaptureData,
    Quit,
    Run,
    Stop,
    ClearDerivedCaches,
    ShowLogicAnalyzer,
    ShowNodeGraph,
    ShowLog,
    ShowMemory,
    ShowWatches,
    ShowTriggers,
    ShowDecoder,
    ResetLaneHeights,
    ResetLayout,
}

/// Host operations requested by UI behavior.
///
/// Implementations belong at the application composition boundary. Hosts that
/// do not provide a capability return an explanatory error or decline the
/// optional picker request.
pub trait HostService {
    /// Installs the wake-up callback used when the host queues a command.
    fn set_command_repaint(&mut self, _repaint: Box<dyn Fn() + Send + Sync>) {}

    /// Drains application commands queued by an optional host shell.
    fn take_commands(&mut self) -> Vec<HostCommand> {
        Vec::new()
    }

    /// Publishes the portable recent-document list to an optional host shell.
    ///
    /// Hosts without a native document menu leave this as a no-op.
    fn publish_recent_files(&self, _paths: &[PathBuf]) {}

    fn choose_open_file(&mut self, request: OpenDialog<'_>) -> Option<PathBuf>;

    fn choose_save_file(&mut self, request: SaveDialog<'_>) -> Option<PathBuf>;

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
    ) -> Result<Option<CacheEntrySnapshot>, String>;
}
