use std::sync::Arc;

use signal_processing::WorkExecutor;

use super::configuration::SigrokFileSourceConfig;
use crate::nodes::sources::synthetic_capture_source::{
    SyntheticCaptureSource, synthetic_presentation,
};
use crate::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};

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
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String>;
}

struct PortableSigrokFileSourceMetadata {
    config: SigrokFileSourceConfig,
}

impl CaptureSourceMetadata for PortableSigrokFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
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

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
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
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        if !config.demo_data() {
            return Err("no Sigrok capture-file acquisition capability was supplied".to_string());
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
