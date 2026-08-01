use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use logic_analyzer_graph_compiler::InlineSourcePreparationExecutor;
use logic_analyzer_ui::{
    AppServices, ApplicationSettings, ApplicationStoragePaths, CacheClearStats, CacheEntrySnapshot,
    HostService, OpenDialog, SaveDialog, default_input_bindings,
};
use node_graph::{FileDialogRequest, FileDialogService};
use signal_processing::{
    CooperativeAppManagerFactory, CooperativeWorkerOperationExecutor, InlineWorkExecutor,
    MemoryArtifactRepository, PersistentStoreConfig, WorkerOperationExecutor,
    portable_worker_kernels,
};

use super::web_worker::WebWorkerAdapter;
use crate::services::PlatformServices;

pub(crate) fn standard_services() -> PlatformServices {
    let worker_operations: Rc<dyn WorkerOperationExecutor> =
        Rc::new(CooperativeWorkerOperationExecutor::new(
            portable_worker_kernels(),
            "browser worker module URLs were not provided",
        ));
    compose_services(worker_operations)
}

pub(crate) fn standard_services_with_worker_urls(
    module_url: &str,
    wasm_url: &str,
) -> PlatformServices {
    let kernels = portable_worker_kernels();
    let required_operations = kernels.operations().cloned().collect::<Vec<_>>();
    let parallelism = browser_parallelism();
    let max_outstanding = parallelism.saturating_mul(2).max(parallelism);
    let worker_operations: Rc<dyn WorkerOperationExecutor> = match WebWorkerAdapter::new(
        module_url,
        wasm_url,
        parallelism,
        max_outstanding,
        &required_operations,
    ) {
        Ok(adapter) => Rc::new(adapter),
        Err(reason) => Rc::new(CooperativeWorkerOperationExecutor::new(kernels, reason)),
    };
    compose_services(worker_operations)
}

fn compose_services(worker_operations: Rc<dyn WorkerOperationExecutor>) -> PlatformServices {
    let work_executor: Arc<dyn signal_processing::WorkExecutor> = Arc::new(InlineWorkExecutor);
    let dsl_file_source_factory =
        logic_analyzer_processing::nodes::sources::dsl_file::unavailable_source_factory();
    let sigrok_file_source_factory =
        logic_analyzer_processing::nodes::sources::sigrok_file::portable_source_factory();
    logic_analyzer_graph_nodes::install_file_source_factories(
        Arc::clone(&dsl_file_source_factory),
        Arc::clone(&sigrok_file_source_factory),
    );
    let ui_services = AppServices::with_host_storage_and_configuration(
        Box::new(WebHostService),
        ApplicationStoragePaths::default(),
        default_input_bindings(),
        ApplicationSettings::default(),
        Vec::new(),
    )
    .with_capture_export_service(logic_analyzer_ui::unavailable_capture_export_service())
    .with_node_file_dialog(Box::new(WebNodeFileDialogService))
    .with_graph_execution_and_builder_overrides(
        Box::new(InlineSourcePreparationExecutor),
        Arc::new(CooperativeAppManagerFactory),
        Arc::clone(&work_executor),
        vec![
            logic_analyzer_graph_nodes::dsl_file_source_runtime_builder_override(
                dsl_file_source_factory,
            ),
            logic_analyzer_graph_nodes::sigrok_file_source_runtime_builder_override(
                sigrok_file_source_factory,
            ),
        ],
    );
    PlatformServices::with_ui_services(
        ui_services,
        Vec::new(),
        Arc::new(MemoryArtifactRepository::new()),
        work_executor,
        worker_operations,
    )
}

fn browser_parallelism() -> usize {
    web_sys::window()
        .map(|window| window.navigator().hardware_concurrency() as usize)
        .unwrap_or(1)
        .saturating_sub(1)
        .clamp(1, 8)
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

#[cfg(test)]
mod web_tests {
    use signal_processing::WorkerExecutionMode;

    use super::standard_services;

    #[test]
    fn web_composition_injects_portable_services_without_native_catalogs() {
        let services = standard_services();
        assert_eq!(services.work_executor().available_parallelism(), 1);
        assert!(!services.artifact_repository().capabilities().durable);
        assert_eq!(
            services.worker_execution_capability().mode(),
            WorkerExecutionMode::Cooperative
        );

        let (ui_services, node_catalogs) = services.into_ui_and_node_catalogs();
        assert!(node_catalogs.is_empty());
        assert_eq!(
            ui_services.worker_execution_capability().mode(),
            WorkerExecutionMode::Cooperative
        );
    }
}
