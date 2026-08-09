use std::sync::Arc;

use platform_runtime::WorkExecutor;
use signal_capture_session::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourceMetadataError, CaptureSourcePresentation,
};
use signal_generators::synthetic_capture_source::{SyntheticCaptureSource, synthetic_presentation};
use signal_runtime::ProcessNodeConstruction;

use super::configuration::SigrokFileSourceConfig;
use crate::CaptureSourceConstructionError;

const LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

/// Platform-neutral construction contract for a Sigrok capture source.
pub trait SigrokFileSourceFactory: Send + Sync {
    /// Returns the lifecycle requirements shared by sources created by this factory.
    fn lifecycle(&self) -> CaptureSourceLifecycle;

    /// Creates lazy source metadata without opening or executing the source.
    ///
    /// # Parameters
    /// - `config`: Persisted source configuration to inspect.
    fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata>;
    /// Creates the executable source and metadata for one configured node.
    ///
    /// # Parameters
    /// - `name`: User-facing node name used by the runtime source.
    /// - `config`: Persisted source configuration to instantiate.
    /// - `work_executor`: Executor used for source work that may be scheduled asynchronously.
    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<
        ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>,
        CaptureSourceConstructionError,
    >;
}

struct PortableSigrokFileSourceMetadata {
    config: SigrokFileSourceConfig,
}

impl CaptureSourceMetadata for PortableSigrokFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn presentation(
        &self,
    ) -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError> {
        Ok(self
            .config
            .demo_data()
            .then(|| synthetic_presentation(self.config.channel_names().iter().cloned(), &[9])))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.demo_data() {
            CaptureSourceCacheIdentity::NotCapture
        } else {
            CaptureSourceCacheIdentity::Dynamic
        }
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, CaptureSourceMetadataError> {
        Ok((!self.config.channel_names().is_empty()).then(|| self.config.channel_names().to_vec()))
    }
}

struct PortableSigrokFileSourceFactory;

impl SigrokFileSourceFactory for PortableSigrokFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(PortableSigrokFileSourceMetadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
        _work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<
        ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>,
        CaptureSourceConstructionError,
    > {
        if !config.demo_data() {
            return Err(CaptureSourceConstructionError::unavailable(
                "no Sigrok capture-file acquisition capability was supplied",
            ));
        }
        let metadata = self.metadata(config.clone());
        Ok(ProcessNodeConstruction::new(
            Box::new(
                SyntheticCaptureSource::new()
                    .with_channel_count(config.channel_count())
                    .with_name(name),
            ),
            metadata,
        ))
    }
}

/// Returns the portable factory for explicit demo data and unavailable file acquisition.
pub fn portable_source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    Arc::new(PortableSigrokFileSourceFactory)
}
