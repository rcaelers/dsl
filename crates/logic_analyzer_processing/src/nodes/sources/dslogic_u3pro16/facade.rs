use std::sync::Arc;

use signal_capture_session::logic_analyzer::LogicCaptureConfig;

use crate::{CaptureSourceLifecycle, CaptureSourceMetadata, ProcessNodeConstruction};

/// Platform-neutral construction contract for a U3Pro16 capture source.
pub trait DsLogicU3Pro16SourceFactory: Send + Sync {
    /// Returns the lifecycle requirements shared by sources created by this factory.
    fn lifecycle(&self) -> CaptureSourceLifecycle;

    /// Creates lazy device-source metadata without starting acquisition.
    ///
    /// # Parameters
    /// - `config`: Requested capture settings to inspect.
    fn metadata(&self, config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata>;
    /// Creates the executable live source and metadata for one configured node.
    ///
    /// # Parameters
    /// - `name`: User-facing node name used by the runtime source.
    /// - `config`: Requested capture settings to instantiate.
    fn create(
        &self,
        name: &str,
        config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String>;
}

/// Returns a source factory that reports the U3Pro16 capability as unavailable.
///
/// Hosts replace this explicit fallback with an adapter-owned implementation
/// through the graph compiler's runtime-builder override contract.
pub fn unavailable_source_factory() -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    Arc::new(UnavailableDsLogicU3Pro16SourceFactory)
}

struct UnavailableDsLogicU3Pro16SourceFactory;

impl DsLogicU3Pro16SourceFactory for UnavailableDsLogicU3Pro16SourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(crate::CaptureSourceKind::Live, false, true, true)
    }

    fn metadata(&self, config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(UnavailableDsLogicU3Pro16Metadata { config })
    }

    fn create(
        &self,
        _name: &str,
        _config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        Err("DSLogic U3Pro16 USB capture is unavailable on this host".into())
    }
}

struct UnavailableDsLogicU3Pro16Metadata {
    config: LogicCaptureConfig,
}

impl CaptureSourceMetadata for UnavailableDsLogicU3Pro16Metadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(crate::CaptureSourceKind::Live, false, true, true)
    }

    fn presentation(&self) -> Result<Option<crate::CaptureSourcePresentation>, String> {
        Ok(Some(crate::CaptureSourcePresentation::Channels(
            self.enabled_channels()
                .enumerate()
                .map(|(viewer_channel, physical_channel)| {
                    (viewer_channel, format!("Ch {physical_channel}"))
                })
                .collect(),
        )))
    }

    fn cache_identity(&self) -> crate::CaptureSourceCacheIdentity {
        crate::CaptureSourceCacheIdentity::NotCapture
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        Ok(Some(
            self.enabled_channels()
                .map(|channel| format!("Ch {channel}"))
                .collect(),
        ))
    }
}

impl UnavailableDsLogicU3Pro16Metadata {
    fn enabled_channels(&self) -> impl Iterator<Item = usize> + '_ {
        (0..u64::BITS as usize).filter(|channel| self.config.input_mask & (1_u64 << channel) != 0)
    }
}
