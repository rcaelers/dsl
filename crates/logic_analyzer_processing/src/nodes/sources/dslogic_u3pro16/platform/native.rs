use std::sync::{Arc, OnceLock};

use signal_processing::logic_analyzer::LogicCaptureConfig;
use signal_processing::{CaptureChannelId, ConfiguredAcquisition};

use super::super::capture::DsLogicU3Pro16Capture;
use super::super::facade::DsLogicU3Pro16SourceFactory;
use super::super::source::DsLogicU3Pro16Source;
use crate::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, CaptureSourceRuntimeCapabilities, ProcessNodeConstruction,
};

const LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true);

struct NativeDsLogicU3Pro16Metadata {
    config: LogicCaptureConfig,
}

impl NativeDsLogicU3Pro16Metadata {
    fn enabled_channels(&self) -> impl Iterator<Item = usize> + '_ {
        (0..u64::BITS as usize).filter(|channel| self.config.input_mask & (1_u64 << channel) != 0)
    }
}

impl CaptureSourceMetadata for NativeDsLogicU3Pro16Metadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        Ok(Some(CaptureSourcePresentation::Channels(
            self.enabled_channels()
                .enumerate()
                .map(|(viewer_channel, physical_channel)| {
                    (viewer_channel, format!("Ch {physical_channel}"))
                })
                .collect(),
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::NotCapture
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        Ok(Some(
            self.enabled_channels()
                .map(|channel| format!("Ch {channel}"))
                .collect(),
        ))
    }

    fn runtime_capabilities(&self) -> CaptureSourceRuntimeCapabilities {
        CaptureSourceRuntimeCapabilities::new(true)
    }

    fn configured_acquisition(&self) -> Result<Option<Box<dyn ConfiguredAcquisition>>, String> {
        let channels = self
            .enabled_channels()
            .map(|channel| CaptureChannelId::new(format!("u3pro16:input:{channel}")))
            .collect::<Vec<_>>();
        DsLogicU3Pro16Capture::new(self.config.clone(), channels)
            .map(|capture| Some(Box::new(capture) as Box<dyn ConfiguredAcquisition>))
            .map_err(|error| error.to_string())
    }
}

struct NativeDsLogicU3Pro16SourceFactory;

impl DsLogicU3Pro16SourceFactory for NativeDsLogicU3Pro16SourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn metadata(&self, config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(NativeDsLogicU3Pro16Metadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        DsLogicU3Pro16Source::open_first(config)
            .map(|source| ProcessNodeConstruction::new(Box::new(source.with_name(name)), metadata))
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn source_factory() -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    static FACTORY: OnceLock<Arc<NativeDsLogicU3Pro16SourceFactory>> = OnceLock::new();
    FACTORY
        .get_or_init(|| Arc::new(NativeDsLogicU3Pro16SourceFactory))
        .clone()
}
