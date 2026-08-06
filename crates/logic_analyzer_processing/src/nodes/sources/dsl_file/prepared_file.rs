use std::path::PathBuf;
use std::sync::Arc;

use platform_artifacts::{ArtifactRepository, PreparedByteSourceOpener};
use platform_runtime::WorkExecutor;
use signal_capture::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, IndexedCapturePresentation,
};

use super::{DslFileSource, DslFileSourceConfig, DslFileSourceFactory};
use crate::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};

const FILE_SOURCE_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct PreparedDslCaptureIndexFactory {
    path: PathBuf,
    opener: Arc<dyn PreparedByteSourceOpener>,
}

impl CaptureIndexFactory for PreparedDslCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.path.display().to_string()
    }

    fn metadata(&self) -> signal_capture::Result<signal_capture::CaptureMetadata> {
        let source = self
            .opener
            .open(&self.path)
            .map_err(|error| signal_capture::Error::ParseError(error.to_string()))?;
        DslFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .metadata()
    }

    fn open(
        self: Box<Self>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> signal_capture::Result<Box<dyn CaptureIndex + Send>> {
        let source = self
            .opener
            .open(&self.path)
            .map_err(|error| signal_capture::Error::ParseError(error.to_string()))?;
        DslFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .open(artifact_repository, work_executor, progress)
    }
}

struct PreparedDslFileSourceMetadata {
    config: DslFileSourceConfig,
    opener: Arc<dyn PreparedByteSourceOpener>,
}

impl CaptureSourceMetadata for PreparedDslFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        let source = self
            .opener
            .open(self.config.path())
            .map_err(|error| error.to_string())?;
        Ok(Some(CaptureSourcePresentation::Indexed(
            IndexedCapturePresentation {
                identity: source.identity(),
                factory: Box::new(PreparedDslCaptureIndexFactory {
                    path: self.config.path().to_owned(),
                    opener: Arc::clone(&self.opener),
                }),
            },
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.path().as_os_str().is_empty() {
            return CaptureSourceCacheIdentity::Dynamic;
        }
        self.opener
            .open(self.config.path())
            .map(|source| CaptureSourceCacheIdentity::Stable(*source.identity().as_bytes()))
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        self.opener
            .open(self.config.path())
            .map_err(|error| error.to_string())
            .and_then(|source| {
                DslFileSource::from_prepared_source(
                    source,
                    self.config.path().display().to_string(),
                )
                .map_err(|error| error.to_string())
            })
            .map(|source| Some(source.header().probe_names.clone()))
    }
}

struct PreparedDslFileSourceFactory {
    opener: Arc<dyn PreparedByteSourceOpener>,
}

impl DslFileSourceFactory for PreparedDslFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(PreparedDslFileSourceMetadata {
            config,
            opener: Arc::clone(&self.opener),
        })
    }

    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        self.opener
            .open(config.path())
            .map_err(|error| error.to_string())
            .and_then(|source| {
                DslFileSource::from_prepared_source(source, config.path().display().to_string())
                    .map_err(|error| error.to_string())
            })
            .map(|source| {
                ProcessNodeConstruction::new(
                    Box::new(
                        source
                            .with_name(name)
                            .with_artifact_repository(artifact_repository)
                            .with_work_executor(work_executor),
                    ),
                    metadata,
                )
            })
    }
}

/// Creates a DSL file-source factory from a host-supplied prepared-file opener.
pub fn prepared_file_source_factory(
    opener: Arc<dyn PreparedByteSourceOpener>,
) -> Arc<dyn DslFileSourceFactory> {
    Arc::new(PreparedDslFileSourceFactory { opener })
}
