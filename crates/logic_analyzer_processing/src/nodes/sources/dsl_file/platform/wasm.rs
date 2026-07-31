use std::sync::{Arc, OnceLock};

use super::super::configuration::DslFileSourceConfig;
use super::super::facade::DslFileSourceFactory;
use crate::nodes::sources::synthetic_capture_source::{
    SyntheticCaptureSource, synthetic_presentation,
};
use crate::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};

const LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct WasmDslFileSourceMetadata {
    config: DslFileSourceConfig,
}

impl CaptureSourceMetadata for WasmDslFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        Ok(Some(synthetic_presentation(
            self.config.channel_names().iter().cloned(),
            &[],
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::Dynamic
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        Ok((!self.config.channel_names().is_empty()).then(|| self.config.channel_names().to_vec()))
    }
}

struct WasmDslFileSourceFactory;

impl DslFileSourceFactory for WasmDslFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(WasmDslFileSourceMetadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
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

pub(crate) fn source_factory() -> Arc<dyn DslFileSourceFactory> {
    static FACTORY: OnceLock<Arc<WasmDslFileSourceFactory>> = OnceLock::new();
    FACTORY
        .get_or_init(|| Arc::new(WasmDslFileSourceFactory))
        .clone()
}
