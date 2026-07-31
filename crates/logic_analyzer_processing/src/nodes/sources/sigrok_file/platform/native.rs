use std::sync::{Arc, OnceLock};

use super::super::configuration::SigrokFileSourceConfig;
use super::super::facade::SigrokFileSourceFactory;
use super::super::implementation::SigrokFileSource;
use crate::nodes::sources::synthetic_capture_source::{
    SyntheticCaptureSource, synthetic_presentation,
};
use crate::support::file_identity_cache::FileIdentityCache;
use crate::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};

const LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct NativeSigrokFileSourceMetadata {
    config: SigrokFileSourceConfig,
    identities: Arc<FileIdentityCache>,
}

impl CaptureSourceMetadata for NativeSigrokFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.demo_data() {
            return Ok(Some(synthetic_presentation(
                self.config.channel_names().iter().cloned(),
                &[9],
            )));
        }
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        Ok(Some(CaptureSourcePresentation::Indexed(
            SigrokFileSource::indexed_capture_presentation(self.config.path()),
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.demo_data() {
            return CaptureSourceCacheIdentity::NotCapture;
        }
        self.identities
            .resolve(self.config.path(), |path| {
                SigrokFileSource::capture_cache_identity(path).map_err(|error| error.to_string())
            })
            .map(CaptureSourceCacheIdentity::Stable)
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        if self.config.demo_data() {
            return Ok(Some(self.config.channel_names().to_vec()));
        }
        SigrokFileSource::new(self.config.path())
            .map(|source| Some(source.header().probe_names.clone()))
            .map_err(|error| error.to_string())
    }
}

struct NativeSigrokFileSourceFactory {
    identities: Arc<FileIdentityCache>,
}

impl SigrokFileSourceFactory for NativeSigrokFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(NativeSigrokFileSourceMetadata {
            config,
            identities: Arc::clone(&self.identities),
        })
    }

    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        let process = if config.demo_data() {
            Box::new(
                SyntheticCaptureSource::new()
                    .with_channel_count(config.channel_count())
                    .with_name(name),
            ) as Box<dyn signal_processing::ProcessNode>
        } else {
            Box::new(
                SigrokFileSource::new(config.path())
                    .map_err(|error| error.to_string())?
                    .with_name(name),
            )
        };
        Ok(ProcessNodeConstruction::new(process, metadata))
    }
}

pub(crate) fn source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    static FACTORY: OnceLock<Arc<NativeSigrokFileSourceFactory>> = OnceLock::new();
    FACTORY
        .get_or_init(|| {
            Arc::new(NativeSigrokFileSourceFactory {
                identities: Arc::new(FileIdentityCache::default()),
            })
        })
        .clone()
}
