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

/// One host-retained output file ready for an explicit user download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadableOutput {
    /// Stable host-local identifier used to request this download.
    pub id: u64,
    /// User-facing filename proposed by the producing graph node.
    pub name: String,
    /// User-visible upstream graph node whose output data was written.
    pub producer_node: String,
    /// User-visible output socket whose data was written.
    pub producer_socket: String,
    /// Number of retained bytes, used for compact UI presentation.
    pub byte_len: u64,
}

/// Host-selected labels for modifier keys shown in portable input hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModifierKeyLabels {
    pub alternate: &'static str,
    pub command: &'static str,
}

impl Default for ModifierKeyLabels {
    fn default() -> Self {
        Self {
            alternate: "Alt",
            command: "Ctrl",
        }
    }
}

/// Presentation and document capabilities supplied by the application host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostUiCapabilities {
    pub direct_document_access: bool,
    pub system_menu_bar: bool,
    pub viewport_close_guard: bool,
    pub modifier_key_labels: ModifierKeyLabels,
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
    /// Returns immutable host UI capabilities selected during composition.
    fn ui_capabilities(&self) -> HostUiCapabilities {
        HostUiCapabilities::default()
    }

    /// Returns runtime-cache diagnostics when the selected data-plane adapter
    /// provides that cache.
    fn decoded_block_cache_snapshot(&self) -> Option<DecodedBlockCacheSnapshot> {
        None
    }

    /// Lists completed output files retained by the host until the user downloads them.
    fn pending_output_downloads(&self) -> Vec<DownloadableOutput> {
        Vec::new()
    }

    /// Starts an explicit download for one previously listed output file.
    ///
    /// # Parameters
    /// - `id`: Stable identifier returned by [`Self::pending_output_downloads`].
    fn download_output(&mut self, _id: u64) -> Result<(), String> {
        Err("output download is unavailable on this host".to_owned())
    }

    /// Installs the wake-up callback used when the host queues a command.
    ///
    /// # Parameters
    /// - `_repaint`: Callback that requests a UI repaint after the host queues a command.
    fn set_command_repaint(&mut self, _repaint: Box<dyn Fn() + Send + Sync>) {}

    /// Drains application commands queued by an optional host shell.
    fn take_commands(&mut self) -> Vec<HostCommand> {
        Vec::new()
    }

    /// Publishes the portable recent-document list to an optional host shell.
    ///
    /// Hosts without a native document menu leave this as a no-op.
    ///
    /// # Parameters
    /// - `_paths`: Recent document paths in most-recent-first order.
    fn publish_recent_files(&self, _paths: &[PathBuf]) {}

    /// Reports whether a previously selected document still exists.
    ///
    /// # Parameters
    /// - `_path`: Host-owned document path to inspect.
    fn document_exists(&self, _path: &Path) -> bool {
        false
    }

    /// Returns a user-facing document name without exposing an opaque host reference.
    ///
    /// # Parameters
    /// - `path`: Host-owned document path to render.
    fn document_display_name(&self, path: &Path) -> String {
        path.display().to_string()
    }

    /// Opens a host file picker for an existing graph document.
    ///
    /// # Parameters
    /// - `request`: Title, filters, and optional starting directory for the picker.
    ///
    /// Returns `None` when the picker is unavailable or the user cancels.
    fn choose_open_file(&mut self, request: OpenDialog<'_>) -> Option<PathBuf>;

    /// Opens a host file picker for a graph-document destination.
    ///
    /// # Parameters
    /// - `request`: Title, suggested name, filters, and optional starting directory.
    ///
    /// Returns `None` when the picker is unavailable or the user cancels.
    fn choose_save_file(&mut self, request: SaveDialog<'_>) -> Option<PathBuf>;

    /// Opens a host directory picker.
    ///
    /// Returns `None` when the picker is unavailable or the user cancels.
    fn choose_directory(&mut self) -> Option<PathBuf>;

    /// Loads and migrates a graph document selected through the host.
    ///
    /// # Parameters
    /// - `path`: File path returned by the host picker or command.
    fn load_graph(&mut self, path: &Path) -> Result<node_graph::GraphState, String>;

    /// Persists a serialized graph document through the host adapter.
    ///
    /// # Parameters
    /// - `path`: Destination file path.
    /// - `graph`: Current graph document serialized by the UI.
    fn save_graph(&mut self, path: &Path, graph: &serde_json::Value) -> Result<(), String>;
}
