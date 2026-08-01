use std::sync::{Arc, OnceLock};

use signal_processing::WorkExecutor;

use super::super::configuration::DslFileSourceConfig;
use super::super::facade::DslFileSourceFactory;
use super::super::implementation::DslFileSource;
use crate::support::file_identity_cache::FileIdentityCache;
use crate::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};

const LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct NativeDslFileSourceMetadata {
    config: DslFileSourceConfig,
    identities: Arc<FileIdentityCache>,
}

impl CaptureSourceMetadata for NativeDslFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        Ok(Some(CaptureSourcePresentation::Indexed(
            DslFileSource::indexed_capture_presentation(self.config.path()),
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.path().as_os_str().is_empty() {
            return CaptureSourceCacheIdentity::Dynamic;
        }
        self.identities
            .resolve(self.config.path(), |path| {
                DslFileSource::capture_cache_identity(path).map_err(|error| error.to_string())
            })
            .map(CaptureSourceCacheIdentity::Stable)
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        DslFileSource::new(self.config.path())
            .map(|source| Some(source.header().probe_names.clone()))
            .map_err(|error| error.to_string())
    }
}

struct NativeDslFileSourceFactory {
    identities: Arc<FileIdentityCache>,
}

impl DslFileSourceFactory for NativeDslFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        LIFECYCLE
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(NativeDslFileSourceMetadata {
            config,
            identities: Arc::clone(&self.identities),
        })
    }

    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        DslFileSource::new(config.path())
            .map(|source| {
                ProcessNodeConstruction::new(
                    Box::new(source.with_name(name).with_work_executor(work_executor)),
                    metadata,
                )
            })
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn source_factory() -> Arc<dyn DslFileSourceFactory> {
    static FACTORY: OnceLock<Arc<NativeDslFileSourceFactory>> = OnceLock::new();
    FACTORY
        .get_or_init(|| {
            Arc::new(NativeDslFileSourceFactory {
                identities: Arc::new(FileIdentityCache::default()),
            })
        })
        .clone()
}
