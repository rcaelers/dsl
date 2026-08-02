use std::path::{Path, PathBuf};

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

/// Host-adapted diagnostics for the shared decoded-block cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecodedBlockCacheSnapshot {
    pub entries: usize,
    pub memory_bytes: usize,
    pub budget_bytes: usize,
    pub hits: u64,
    pub misses: u64,
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
    /// Returns runtime-cache diagnostics when the selected data-plane adapter
    /// provides that cache.
    fn decoded_block_cache_snapshot(&self) -> Option<DecodedBlockCacheSnapshot> {
        None
    }

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
}
