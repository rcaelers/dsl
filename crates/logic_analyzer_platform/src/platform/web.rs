use std::rc::Rc;
use std::sync::Arc;

use signal_artifacts::MemoryArtifactRepository;
use signal_derived::portable_worker_kernels;
use signal_runtime::{
    CooperativeAppManagerFactory, CooperativeWorkerOperationExecutor, InlineWorkExecutor,
    WorkerOperationExecutor,
};

use super::web_artifact_repository::BrowserArtifactRepository;
use super::web_capture_worker::install_capture_worker;
use super::web_file_import::{
    BrowserFilePickerService, BrowserFileRegistry, dsl_source_factory, sigrok_source_factory,
};
use super::web_worker::WebWorkerAdapter;
use crate::services::{PlatformServices, WorkerGraphHostServices};

/// Returns browser-worker storage and source factories for application-owned node composition.
pub fn worker_graph_host_services() -> WorkerGraphHostServices {
    let (dsl_file_source_factory, sigrok_file_source_factory) =
        super::web_file_import::worker_file_source_factories();
    WorkerGraphHostServices {
        output_storage: super::web_output_storage::output_storage(),
        dsl_file_source_factory,
        sigrok_file_source_factory,
    }
}

pub(crate) fn standard_services(_application_id: &str) -> PlatformServices {
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
    _application_id: &str,
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
    let work_executor: Arc<dyn signal_runtime::WorkExecutor> = Arc::new(InlineWorkExecutor);
    let imported_files = Arc::new(BrowserFileRegistry::default());
    let capture_worker = worker_clients
        .as_ref()
        .map(|clients| Arc::clone(&clients.capture));
    let graph_worker = worker_clients.map(|clients| clients.graph);
    let dsl_file_source_factory =
        dsl_source_factory(Arc::clone(&imported_files), capture_worker.clone());
    let sigrok_file_source_factory =
        sigrok_source_factory(Arc::clone(&imported_files), capture_worker.clone());
    PlatformServices {
        capture_worker_client: capture_worker,
        app_manager_factory: Arc::new(CooperativeAppManagerFactory),
        dsl_file_source_factory,
        sigrok_file_source_factory,
        sigrok_decoder_runtime: None,
        sigrok_catalog_scanner: None,
        u3pro16_source_factory: None,
        output_storage: None,
        file_picker: Some(Box::new(BrowserFilePickerService::new(imported_files))),
        graph_worker_client: graph_worker,
        artifact_repository,
        work_executor,
        worker_operation_executor: worker_operations,
    }
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
    use signal_runtime::WorkerExecutionMode;

    use super::standard_services;

    #[test]
    fn web_composition_injects_portable_services_without_native_catalogs() {
        let services = standard_services("test-application");
        assert_eq!(services.work_executor().available_parallelism(), 1);
        assert!(!services.artifact_repository().capabilities().durable);
        assert_eq!(
            services.worker_execution_capability().mode(),
            WorkerExecutionMode::Cooperative
        );
    }
}
