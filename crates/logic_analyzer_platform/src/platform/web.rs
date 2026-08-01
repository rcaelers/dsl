use std::path::{Path, PathBuf};
use std::sync::Arc;

use logic_analyzer_graph_compiler::InlineSourcePreparationExecutor;
use logic_analyzer_ui::{
    AppServices, ApplicationSettings, ApplicationStoragePaths, CacheClearStats, CacheEntrySnapshot,
    HostService, OpenDialog, SaveDialog, default_input_bindings,
};
use node_graph::{FileDialogRequest, FileDialogService};
use signal_processing::{
    CooperativeAppManagerFactory, InlineWorkExecutor, MemoryArtifactRepository,
    PersistentStoreConfig,
};

use crate::services::PlatformServices;

pub(crate) fn standard_services() -> PlatformServices {
    let work_executor: Arc<dyn signal_processing::WorkExecutor> = Arc::new(InlineWorkExecutor);
    let ui_services = AppServices::with_host_storage_and_configuration(
        Box::new(WebHostService),
        ApplicationStoragePaths::default(),
        default_input_bindings(),
        ApplicationSettings::default(),
        Vec::new(),
    )
    .with_node_file_dialog(Box::new(WebNodeFileDialogService))
    .with_graph_execution(
        Box::new(InlineSourcePreparationExecutor),
        Arc::new(CooperativeAppManagerFactory),
        Arc::clone(&work_executor),
    );
    PlatformServices::with_ui_services(
        ui_services,
        Arc::new(MemoryArtifactRepository::new()),
        work_executor,
    )
}

struct WebHostService;
struct WebNodeFileDialogService;

impl FileDialogService for WebNodeFileDialogService {
    fn available(&self) -> bool {
        false
    }

    fn pick(&mut self, _request: FileDialogRequest<'_>) -> Option<String> {
        None
    }
}

impl HostService for WebHostService {
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
    "this web host does not provide file or persistent-cache access".into()
}
