use std::rc::Rc;
use std::sync::Arc;

use logic_analyzer_graph_runtime::{
    CaptureWorkerSourcePreparationExecutor, InlineSourcePreparationExecutor,
    SourcePreparationExecutor,
};
use logic_analyzer_ui::{AppServices, ApplicationSettings, default_input_bindings};
use signal_artifacts::MemoryArtifactRepository;
use signal_processing::{
    CooperativeAppManagerFactory, CooperativeWorkerOperationExecutor, InlineWorkExecutor,
    WorkerOperationExecutor, portable_worker_kernels,
};

use super::web_artifact_repository::BrowserArtifactRepository;
use super::web_capture_worker::install_capture_worker;
use super::web_document::BrowserDocumentHostService;
use super::web_file_import::{
    BrowserFileRegistry, BrowserNodeFileDialogService, dsl_source_factory, sigrok_source_factory,
};
use super::web_worker::WebWorkerAdapter;
use crate::services::PlatformServices;

pub(crate) fn standard_services() -> PlatformServices {
    let worker_operations: Rc<dyn WorkerOperationExecutor> =
        Rc::new(CooperativeWorkerOperationExecutor::new(
            portable_worker_kernels(),
            "browser worker module URLs were not provided",
        ));
    compose_services(
        worker_operations,
        Arc::new(MemoryArtifactRepository::new()),
        None,
    )
}

pub(crate) async fn standard_services_with_worker_urls(
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
    let artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository> =
        match BrowserArtifactRepository::open().await {
            Ok(repository) => Arc::new(repository),
            Err(error) => {
                tracing::warn!(%error, "browser persistence is unavailable; using the memory repository");
                Arc::new(MemoryArtifactRepository::new())
            }
        };
    let worker_clients = match install_capture_worker(
        module_url,
        wasm_url,
        32,
        Arc::clone(&artifact_repository),
    ) {
        Ok(clients) => Some(clients),
        Err(error) => {
            tracing::warn!(%error, "browser capture worker is unavailable; using inline source preparation");
            None
        }
    };
    compose_services(worker_operations, artifact_repository, worker_clients)
}

fn compose_services(
    worker_operations: Rc<dyn WorkerOperationExecutor>,
    artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository>,
    worker_clients: Option<super::web_capture_worker::BrowserWorkerClients>,
) -> PlatformServices {
    let work_executor: Arc<dyn signal_processing::WorkExecutor> = Arc::new(InlineWorkExecutor);
    let imported_files = Arc::new(BrowserFileRegistry::default());
    let capture_worker = worker_clients
        .as_ref()
        .map(|clients| Arc::clone(&clients.capture));
    let graph_worker = worker_clients.map(|clients| clients.graph);
    let dsl_file_source_factory =
        dsl_source_factory(Arc::clone(&imported_files), capture_worker.clone());
    let sigrok_file_source_factory =
        sigrok_source_factory(Arc::clone(&imported_files), capture_worker.clone());
    logic_analyzer_graph_nodes::install_file_source_factories(
        Arc::clone(&dsl_file_source_factory),
        Arc::clone(&sigrok_file_source_factory),
    );
    let source_preparation_executor: Box<dyn SourcePreparationExecutor> =
        if let Some(client) = capture_worker {
            Box::new(CaptureWorkerSourcePreparationExecutor::new(
                client,
                Box::new(InlineSourcePreparationExecutor),
            ))
        } else {
            Box::new(InlineSourcePreparationExecutor)
        };
    let ui_services = AppServices::with_host_configuration(
        Box::new(BrowserDocumentHostService::new()),
        default_input_bindings(),
        ApplicationSettings::default(),
        Vec::new(),
    )
    .with_capture_export_service(logic_analyzer_ui::unavailable_capture_export_service())
    .with_node_file_dialog(Box::new(BrowserNodeFileDialogService::new(imported_files)))
    .with_graph_execution_and_builder_overrides(
        source_preparation_executor,
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
    )
    .with_graph_worker_client(graph_worker);
    PlatformServices::with_ui_services(
        ui_services,
        Vec::new(),
        artifact_repository,
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
