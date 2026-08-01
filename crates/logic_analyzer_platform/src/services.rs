use std::sync::Arc;

use logic_analyzer_ui::AppServices;
use signal_processing::{ArtifactRepository, WorkExecutor};

/// Opaque host services assembled for one application instance.
///
/// The bundle grows as storage, execution, source-acquisition, and export
/// ports become injectable. Its fields remain private so applications compose
/// through supported owner contracts rather than concrete platform types.
pub struct PlatformServices {
    ui_services: AppServices,
    artifact_repository: Arc<dyn ArtifactRepository>,
    work_executor: Arc<dyn WorkExecutor>,
}

impl PlatformServices {
    pub(crate) fn with_ui_services(
        ui_services: AppServices,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        Self {
            ui_services,
            artifact_repository,
            work_executor,
        }
    }

    /// Returns the UI-owned services for application construction.
    pub fn into_ui_services(self) -> AppServices {
        self.ui_services
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

    /// Decomposes the platform bundle for application composition.
    pub fn into_parts(
        self,
    ) -> (
        AppServices,
        Arc<dyn ArtifactRepository>,
        Arc<dyn WorkExecutor>,
    ) {
        (
            self.ui_services,
            self.artifact_repository,
            self.work_executor,
        )
    }
}
