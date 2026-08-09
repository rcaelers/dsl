use std::sync::Arc;

use logic_analyzer_acquisition::LogicCaptureConfig;
use signal_capture::CaptureChannelId;
use signal_capture_session::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourceMetadataError, CaptureSourcePresentation, CaptureSourceRuntimeCapabilities,
    ConfiguredAcquisition,
};
use signal_runtime::ProcessNodeConstruction;

use super::{
    DsLogicU3Pro16Capture, DsLogicU3Pro16Source, DsLogicU3Pro16SourceError,
    DsLogicU3Pro16SourceFactory, DsLogicU3Pro16TransportFactory,
};

const U3PRO16_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true);

struct HostU3Pro16SourceFactory {
    transport_factory: Arc<dyn DsLogicU3Pro16TransportFactory>,
}

impl DsLogicU3Pro16SourceFactory for HostU3Pro16SourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        U3PRO16_LIFECYCLE
    }

    fn metadata(&self, config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(HostU3Pro16Metadata {
            config,
            transport_factory: Arc::clone(&self.transport_factory),
        })
    }

    fn create(
        &self,
        name: &str,
        config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, DsLogicU3Pro16SourceError>
    {
        let metadata = self.metadata(config.clone());
        self.transport_factory
            .open()
            .and_then(|transport| DsLogicU3Pro16Source::from_transport(config, transport))
            .map(|source| ProcessNodeConstruction::new(Box::new(source.with_name(name)), metadata))
            .map_err(DsLogicU3Pro16SourceError::from)
    }
}

struct HostU3Pro16Metadata {
    config: LogicCaptureConfig,
    transport_factory: Arc<dyn DsLogicU3Pro16TransportFactory>,
}

impl HostU3Pro16Metadata {
    fn enabled_channels(&self) -> impl Iterator<Item = usize> + '_ {
        (0..u64::BITS as usize).filter(|channel| self.config.input_mask & (1_u64 << channel) != 0)
    }
}

impl CaptureSourceMetadata for HostU3Pro16Metadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        U3PRO16_LIFECYCLE
    }

    fn presentation(
        &self,
    ) -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError> {
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

    fn channel_names(&self) -> Result<Option<Vec<String>>, CaptureSourceMetadataError> {
        Ok(Some(
            self.enabled_channels()
                .map(|channel| format!("Ch {channel}"))
                .collect(),
        ))
    }

    fn runtime_capabilities(&self) -> CaptureSourceRuntimeCapabilities {
        CaptureSourceRuntimeCapabilities::new(true)
    }

    fn configured_acquisition(
        &self,
    ) -> Result<Option<Box<dyn ConfiguredAcquisition>>, CaptureSourceMetadataError> {
        let channels = self
            .enabled_channels()
            .map(|channel| CaptureChannelId::new(format!("u3pro16:input:{channel}")))
            .collect::<Vec<_>>();
        DsLogicU3Pro16Capture::new(
            self.config.clone(),
            channels,
            Arc::clone(&self.transport_factory),
        )
        .map(|capture| Some(Box::new(capture) as Box<dyn ConfiguredAcquisition>))
        .map_err(CaptureSourceMetadataError::acquisition)
    }
}

/// Creates the U3Pro16 source adapter for an injected host USB transport.
pub fn source_factory(
    transport_factory: Arc<dyn DsLogicU3Pro16TransportFactory>,
) -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    Arc::new(HostU3Pro16SourceFactory { transport_factory })
}
