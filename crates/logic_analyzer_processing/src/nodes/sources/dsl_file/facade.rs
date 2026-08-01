use std::sync::Arc;

use signal_processing::{ArtifactRepository, WorkExecutor};

use super::configuration::DslFileSourceConfig;
use crate::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};

const LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

/// Platform-neutral construction contract for a DSL capture source.
pub trait DslFileSourceFactory: Send + Sync {
    fn lifecycle(&self) -> CaptureSourceLifecycle;
    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata>;
    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String>;
}

struct UnavailableDslFileSourceMetadata {
    config: DslFileSourceConfig,
}

impl CaptureSourceMetadata for UnavailableDslFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        Ok(None)
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::Dynamic
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        Ok((!self.config.channel_names().is_empty()).then(|| self.config.channel_names().to_vec()))
    }
}

struct UnavailableDslFileSourceFactory;

impl DslFileSourceFactory for UnavailableDslFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(UnavailableDslFileSourceMetadata { config })
    }

    fn create(
        &self,
        _name: &str,
        _config: DslFileSourceConfig,
        _artifact_repository: Arc<dyn ArtifactRepository>,
        _work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        Err("no DSL capture-file acquisition capability was supplied".to_string())
    }
}

/// Returns a factory that reports absent DSL file acquisition explicitly.
pub fn unavailable_source_factory() -> Arc<dyn DslFileSourceFactory> {
    Arc::new(UnavailableDslFileSourceFactory)
}
