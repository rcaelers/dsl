use std::rc::Rc;
use std::sync::Arc;

use logic_analyzer_graph_api::node::DirectoryNodeCatalog;
use logic_analyzer_ui::AppServices;
use signal_processing::{
    ArtifactRepository, WorkExecutor, WorkerExecutionCapability, WorkerOperationExecutor,
};

/// Opaque host services assembled for one application instance.
///
/// The bundle grows as storage, execution, source-acquisition, and export
/// ports become injectable. Its fields remain private so applications compose
/// through supported owner contracts rather than concrete platform types.
pub struct PlatformServices {
    ui_services: AppServices,
    node_catalogs: Vec<Box<dyn DirectoryNodeCatalog>>,
    artifact_repository: Arc<dyn ArtifactRepository>,
    work_executor: Arc<dyn WorkExecutor>,
    worker_operation_executor: Rc<dyn WorkerOperationExecutor>,
}

impl PlatformServices {
    pub(crate) fn with_ui_services(
        ui_services: AppServices,
        node_catalogs: Vec<Box<dyn DirectoryNodeCatalog>>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        worker_operation_executor: Rc<dyn WorkerOperationExecutor>,
    ) -> Self {
        let ui_services =
            ui_services.with_worker_operation_executor(Rc::clone(&worker_operation_executor));
        Self {
            ui_services,
            node_catalogs,
            artifact_repository,
            work_executor,
            worker_operation_executor,
        }
    }

    /// Returns the UI-owned services for application construction.
    pub fn into_ui_services(self) -> AppServices {
        self.ui_services
    }

    /// Returns the UI services and host-selected dynamic node catalogs.
    pub fn into_ui_and_node_catalogs(self) -> (AppServices, Vec<Box<dyn DirectoryNodeCatalog>>) {
        (self.ui_services, self.node_catalogs)
    }

    /// Returns the host-selected repository for generated capture and derived
    /// data artifacts.
    pub fn artifact_repository(&self) -> Arc<dyn ArtifactRepository> {
        Arc::clone(&self.artifact_repository)
    }

    /// Returns the host-selected bounded executor for portable processing work.
    pub fn work_executor(&self) -> Arc<dyn WorkExecutor> {
        Arc::clone(&self.work_executor)
    }

    /// Returns the host selected for finite serializable operations.
    pub fn worker_operation_executor(&self) -> Rc<dyn WorkerOperationExecutor> {
        Rc::clone(&self.worker_operation_executor)
    }

    /// Describes whether parallel finite-operation execution is available.
    pub fn worker_execution_capability(&self) -> WorkerExecutionCapability {
        self.worker_operation_executor.capability()
    }
}
