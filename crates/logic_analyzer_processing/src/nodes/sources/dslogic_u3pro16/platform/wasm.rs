use std::sync::{Arc, OnceLock};

use signal_processing::logic_analyzer::LogicCaptureConfig;

use super::super::facade::DsLogicU3Pro16SourceFactory;
use crate::nodes::sources::synthetic_capture_source::{
    SyntheticCaptureSource, synthetic_presentation,
};
use crate::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};

const LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true);

struct WasmDsLogicU3Pro16Metadata {
    config: LogicCaptureConfig,
}

impl WasmDsLogicU3Pro16Metadata {
    fn enabled_channel_names(&self) -> Vec<String> {
        (0..u64::BITS as usize)
            .filter(|channel| self.config.input_mask & (1_u64 << channel) != 0)
            .map(|channel| format!("Ch {channel}"))
            .collect()
    }
}

impl CaptureSourceMetadata for WasmDsLogicU3Pro16Metadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        Ok(Some(synthetic_presentation(
            self.enabled_channel_names(),
            &[],
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::NotCapture
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        Ok(Some(self.enabled_channel_names()))
    }
}

struct WasmDsLogicU3Pro16SourceFactory;

impl DsLogicU3Pro16SourceFactory for WasmDsLogicU3Pro16SourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn metadata(&self, config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(WasmDsLogicU3Pro16Metadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        Ok(ProcessNodeConstruction::new(
            Box::new(
                SyntheticCaptureSource::new()
                    .with_channel_count(config.input_mask.count_ones() as usize)
                    .with_name(name),
            ),
            metadata,
        ))
    }
}

pub(crate) fn source_factory() -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    static FACTORY: OnceLock<Arc<WasmDsLogicU3Pro16SourceFactory>> = OnceLock::new();
    FACTORY
        .get_or_init(|| Arc::new(WasmDsLogicU3Pro16SourceFactory))
        .clone()
}
