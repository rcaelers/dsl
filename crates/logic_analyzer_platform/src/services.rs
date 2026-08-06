//! Host-selected service implementations supplied to an application composition root.

use std::rc::Rc;
use std::sync::Arc;

use logic_analyzer_graph_orchestration::GraphWorkerClient;
use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokCatalogScanner, SigrokDecoderRuntime,
};
use logic_analyzer_processing::nodes::sinks::OutputStorage;
use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSourceFactory;
use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::DsLogicU3Pro16SourceFactory;
use logic_analyzer_processing::nodes::sources::sigrok_file::SigrokFileSourceFactory;
use signal_artifacts::ArtifactRepository;
use signal_capture::CaptureWorkerClient;
use signal_runtime::{
    AppManagerFactory, WorkExecutor, WorkerExecutionCapability, WorkerOperationExecutor,
};

use crate::FilePickerService;

/// Browser-worker host resources adapted to concrete graph nodes by `app_web`.
pub struct WorkerGraphHostServices {
    pub output_storage: Arc<dyn OutputStorage>,
    pub dsl_file_source_factory: Arc<dyn DslFileSourceFactory>,
    pub sigrok_file_source_factory: Arc<dyn SigrokFileSourceFactory>,
}

/// Host implementations selected for one application instance.
///
/// This is an intermediate composition record: application roots consume its
/// fields to select concrete nodes and build `AppServices`. Domain-aware fields
/// move to their owners as the platform-neutral capability boundary is
/// completed.
pub struct PlatformServices {
    pub capture_worker_client: Option<Arc<CaptureWorkerClient>>,
    pub app_manager_factory: Arc<dyn AppManagerFactory>,
    pub dsl_file_source_factory: Arc<dyn DslFileSourceFactory>,
    pub sigrok_file_source_factory: Arc<dyn SigrokFileSourceFactory>,
    pub sigrok_decoder_runtime: Option<Arc<dyn SigrokDecoderRuntime>>,
    pub sigrok_catalog_scanner: Option<Arc<dyn SigrokCatalogScanner>>,
    pub u3pro16_source_factory: Option<Arc<dyn DsLogicU3Pro16SourceFactory>>,
    pub output_storage: Option<Arc<dyn OutputStorage>>,
    pub file_picker: Option<Box<dyn FilePickerService>>,
    pub graph_worker_client: Option<Arc<GraphWorkerClient>>,
    pub artifact_repository: Arc<dyn ArtifactRepository>,
    pub work_executor: Arc<dyn WorkExecutor>,
    pub worker_operation_executor: Rc<dyn WorkerOperationExecutor>,
}

impl PlatformServices {
    /// Returns the host-selected artifact repository.
    pub fn artifact_repository(&self) -> Arc<dyn ArtifactRepository> {
        Arc::clone(&self.artifact_repository)
    }

    /// Returns the host-selected bounded work executor.
    pub fn work_executor(&self) -> Arc<dyn WorkExecutor> {
        Arc::clone(&self.work_executor)
    }

    /// Describes whether parallel finite-operation execution is available.
    pub fn worker_execution_capability(&self) -> WorkerExecutionCapability {
        self.worker_operation_executor.capability()
    }
}
