use std::sync::Arc;

use logic_analyzer_graph_orchestration::GraphWorkerClient;
use logic_analyzer_processing::nodes::sinks::OutputStorage;
use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSourceFactory;
use logic_analyzer_processing::nodes::sources::sigrok_file::SigrokFileSourceFactory;
use signal_artifacts::ArtifactRepository;
use signal_capture::CaptureWorkerClient;

use super::web_artifact_repository::BrowserArtifactRepository;
use super::web_capture_worker::install_capture_worker;
use super::web_file_import::{
    BrowserFilePickerService, BrowserFileRegistry, dsl_source_factory, sigrok_source_factory,
    worker_dsl_file_source_factory, worker_sigrok_file_source_factory,
};
use crate::FilePickerService;

/// Shared browser state for one application's imported capture files.
///
/// The handle keeps the file picker and the two format adapters on the same
/// opaque-reference registry without selecting any application services.
#[derive(Default)]
pub struct BrowserFileImport {
    registry: Arc<BrowserFileRegistry>,
}

impl BrowserFileImport {
    /// Creates an empty browser capture-file registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a browser file picker backed by this registry.
    pub fn file_picker(&self) -> Box<dyn FilePickerService> {
        Box::new(BrowserFilePickerService::new(Arc::clone(&self.registry)))
    }

    /// Creates the DSL source adapter backed by this registry.
    pub fn dsl_file_source_factory(
        &self,
        capture_worker: Option<Arc<CaptureWorkerClient>>,
    ) -> Arc<dyn DslFileSourceFactory> {
        dsl_source_factory(Arc::clone(&self.registry), capture_worker)
    }

    /// Creates the Sigrok source adapter backed by this registry.
    pub fn sigrok_file_source_factory(
        &self,
        capture_worker: Option<Arc<CaptureWorkerClient>>,
    ) -> Arc<dyn SigrokFileSourceFactory> {
        sigrok_source_factory(Arc::clone(&self.registry), capture_worker)
    }
}

/// Opens the browser's durable artifact repository.
pub async fn open_browser_artifact_repository() -> Result<Arc<dyn ArtifactRepository>, String> {
    BrowserArtifactRepository::open()
        .await
        .map(|repository| Arc::new(repository) as Arc<dyn ArtifactRepository>)
}

/// Starts the shared capture/graph browser worker and returns its two clients.
pub fn browser_worker_clients(
    module_url: &str,
    wasm_url: &str,
    max_outstanding_capture_requests: usize,
    artifact_repository: Arc<dyn ArtifactRepository>,
) -> Result<(Arc<CaptureWorkerClient>, Arc<GraphWorkerClient>), String> {
    install_capture_worker(
        module_url,
        wasm_url,
        max_outstanding_capture_requests,
        artifact_repository,
    )
    .map(|clients| (clients.capture, clients.graph))
}

/// Creates the DSL source adapter used inside the graph worker.
pub fn browser_worker_dsl_file_source_factory() -> Arc<dyn DslFileSourceFactory> {
    worker_dsl_file_source_factory()
}

/// Creates the Sigrok source adapter used inside the graph worker.
pub fn browser_worker_sigrok_file_source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    worker_sigrok_file_source_factory()
}

/// Returns the graph worker's browser-backed output destination.
pub fn browser_worker_output_storage() -> Arc<dyn OutputStorage> {
    super::web_output_storage::output_storage()
}

/// Chooses a conservative browser worker count from host concurrency.
pub fn browser_worker_parallelism() -> usize {
    web_sys::window()
        .map(|window| window.navigator().hardware_concurrency() as usize)
        .unwrap_or(1)
        .saturating_sub(1)
        .clamp(1, 8)
}
