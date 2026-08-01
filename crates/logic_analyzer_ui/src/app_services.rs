use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use input_bindings::InputBindings;
use logic_analyzer_graph_api::node::RuntimeBuilderOverride;
use logic_analyzer_graph_compiler::SourcePreparationExecutor;
use node_graph::FileDialogService;
use signal_processing::{
    AppManagerFactory, ArtifactRepository, CooperativeWorkerOperationExecutor, InlineWorkExecutor,
    MemoryArtifactRepository, PersistentStoreConfig, WorkExecutor, WorkerOperationExecutor,
    portable_worker_kernels,
};

use crate::application_settings::{ApplicationSettings, default_input_bindings};
use crate::capture_export_service::{CaptureExportService, unavailable_capture_export_service};
use crate::graph_service::{
    GraphService, graph_service_with_execution, graph_service_with_execution_and_builder_overrides,
    standard_graph_service,
};
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
    work_executor: Arc<dyn WorkExecutor>,
    worker_operation_executor: Rc<dyn WorkerOperationExecutor>,
    capture_export_service: Box<dyn CaptureExportService>,
    artifact_repository: Arc<dyn ArtifactRepository>,
}

pub(crate) struct AppServiceParts {
    pub(crate) graph_service: Box<dyn GraphService>,
    pub(crate) host_service: Box<dyn HostService>,
    pub(crate) storage_paths: ApplicationStoragePaths,
    pub(crate) input_bindings: InputBindings,
    pub(crate) application_settings: ApplicationSettings,
    pub(crate) host_symbol_fonts: Vec<egui::FontData>,
    pub(crate) node_file_dialog: Option<Box<dyn FileDialogService>>,
    pub(crate) work_executor: Arc<dyn WorkExecutor>,
    pub(crate) worker_operation_executor: Rc<dyn WorkerOperationExecutor>,
    pub(crate) capture_export_service: Box<dyn CaptureExportService>,
    pub(crate) artifact_repository: Arc<dyn ArtifactRepository>,
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
        let artifact_repository: Arc<dyn ArtifactRepository> =
            Arc::new(MemoryArtifactRepository::new());
        let mut graph_service = standard_graph_service();
        graph_service.set_artifact_repository(Arc::clone(&artifact_repository));
        Self {
            graph_service,
            host_service,
            storage_paths,
            input_bindings,
            application_settings,
            host_symbol_fonts,
            node_file_dialog: None,
            work_executor: Arc::new(InlineWorkExecutor),
            worker_operation_executor: Rc::new(CooperativeWorkerOperationExecutor::new(
                portable_worker_kernels(),
                "no parallel finite-operation host was supplied",
            )),
            capture_export_service: unavailable_capture_export_service(),
            artifact_repository,
        }
    }

    /// Supplies the host destination and execution adapter for capture export.
    pub fn with_capture_export_service(mut self, service: Box<dyn CaptureExportService>) -> Self {
        self.capture_export_service = service;
        self
    }

    /// Supplies the artifact repository shared by compiler-owned and capture stores.
    pub fn with_artifact_repository(mut self, repository: Arc<dyn ArtifactRepository>) -> Self {
        self.graph_service
            .set_artifact_repository(Arc::clone(&repository));
        self.artifact_repository = repository;
        self
    }

    /// Supplies the host capability used by file controls embedded in graph nodes.
    pub fn with_node_file_dialog(mut self, service: Box<dyn FileDialogService>) -> Self {
        self.node_file_dialog = Some(service);
        self
    }

    /// Retains the host selected for finite serializable operations.
    pub fn with_worker_operation_executor(
        mut self,
        executor: Rc<dyn WorkerOperationExecutor>,
    ) -> Self {
        self.worker_operation_executor = executor;
        self
    }

    /// Reports the finite-operation capability retained by the application.
    pub fn worker_execution_capability(&self) -> signal_processing::WorkerExecutionCapability {
        self.worker_operation_executor.capability()
    }

    /// Replaces portable graph execution with host-selected adapters.
    pub fn with_graph_execution(
        mut self,
        source_preparation_executor: Box<dyn SourcePreparationExecutor>,
        runtime_factory: std::sync::Arc<dyn AppManagerFactory>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        self.graph_service = graph_service_with_execution(
            source_preparation_executor,
            runtime_factory,
            Arc::clone(&work_executor),
        );
        self.graph_service
            .set_artifact_repository(Arc::clone(&self.artifact_repository));
        self.work_executor = work_executor;
        self
    }

    /// Replaces graph execution with host-selected adapters and node factories.
    pub fn with_graph_execution_and_builder_overrides(
        mut self,
        source_preparation_executor: Box<dyn SourcePreparationExecutor>,
        runtime_factory: std::sync::Arc<dyn AppManagerFactory>,
        work_executor: Arc<dyn WorkExecutor>,
        builder_overrides: Vec<RuntimeBuilderOverride>,
    ) -> Self {
        self.graph_service = graph_service_with_execution_and_builder_overrides(
            source_preparation_executor,
            runtime_factory,
            Arc::clone(&work_executor),
            builder_overrides,
        );
        self.graph_service
            .set_artifact_repository(Arc::clone(&self.artifact_repository));
        self.work_executor = work_executor;
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
            work_executor: self.work_executor,
            worker_operation_executor: self.worker_operation_executor,
            capture_export_service: self.capture_export_service,
            artifact_repository: self.artifact_repository,
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
}

impl ApplicationStoragePaths {
    pub fn new(derived_cache_directory: Option<PathBuf>) -> Self {
        Self {
            derived_cache_directory,
        }
    }

    pub fn derived_cache_directory(&self) -> Option<&Path> {
        self.derived_cache_directory.as_deref()
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

        let configured = ApplicationStoragePaths::new(Some("cache/derived".into()));
        assert_eq!(
            configured.derived_cache_directory(),
            Some(Path::new("cache/derived"))
        );
    }
}
