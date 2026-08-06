use std::path::PathBuf;
use std::sync::Arc;

use platform_artifacts::{ArtifactRepository, PreparedByteSourceOpener};
use platform_runtime::WorkExecutor;
use signal_capture::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, IndexedCapturePresentation,
};
use signal_capture_session::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation,
};
use signal_generators::synthetic_capture_source::SyntheticCaptureSource;
use signal_runtime::{ProcessNode, ProcessNodeConstruction};

use super::{
    SigrokFileSource, SigrokFileSourceConfig, SigrokFileSourceFactory, portable_source_factory,
};

const FILE_SOURCE_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct PreparedSigrokCaptureIndexFactory {
    path: PathBuf,
    opener: Arc<dyn PreparedByteSourceOpener>,
}

impl CaptureIndexFactory for PreparedSigrokCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.path.display().to_string()
    }

    fn metadata(&self) -> signal_capture::Result<signal_capture::CaptureMetadata> {
        let source = self
            .opener
            .open(&self.path)
            .map_err(|error| signal_capture::Error::ParseError(error.to_string()))?;
        SigrokFileSource::indexed_capture_presentation(source, self.path.display().to_string())
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
        SigrokFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .open(artifact_repository, work_executor, progress)
    }
}

struct PreparedSigrokFileSourceMetadata {
    config: SigrokFileSourceConfig,
    opener: Arc<dyn PreparedByteSourceOpener>,
}

impl CaptureSourceMetadata for PreparedSigrokFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.demo_data() {
            return portable_source_factory()
                .metadata(self.config.clone())
                .presentation();
        }
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
                factory: Box::new(PreparedSigrokCaptureIndexFactory {
                    path: self.config.path().to_owned(),
                    opener: Arc::clone(&self.opener),
                }),
            },
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.demo_data() {
            return CaptureSourceCacheIdentity::NotCapture;
        }
        self.opener
            .open(self.config.path())
            .map(|source| CaptureSourceCacheIdentity::Stable(*source.identity().as_bytes()))
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        if self.config.demo_data() {
            return Ok(Some(self.config.channel_names().to_vec()));
        }
        self.opener
            .open(self.config.path())
            .map_err(|error| error.to_string())
            .and_then(|source| {
                SigrokFileSource::from_prepared_source(source).map_err(|error| error.to_string())
            })
            .map(|source| Some(source.header().probe_names.clone()))
    }
}

struct PreparedSigrokFileSourceFactory {
    opener: Arc<dyn PreparedByteSourceOpener>,
}

impl SigrokFileSourceFactory for PreparedSigrokFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(PreparedSigrokFileSourceMetadata {
            config,
            opener: Arc::clone(&self.opener),
        })
    }

    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        let process = if config.demo_data() {
            Box::new(
                SyntheticCaptureSource::new()
                    .with_channel_count(config.channel_count())
                    .with_name(name),
            ) as Box<dyn ProcessNode>
        } else {
            Box::new(
                SigrokFileSource::from_prepared_source(
                    self.opener
                        .open(config.path())
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?
                .with_name(name)
                .with_work_executor(work_executor),
            )
        };
        Ok(ProcessNodeConstruction::new(process, metadata))
    }
}

/// Creates a Sigrok file-source factory from a host-supplied prepared-file opener.
pub fn prepared_file_source_factory(
    opener: Arc<dyn PreparedByteSourceOpener>,
) -> Arc<dyn SigrokFileSourceFactory> {
    Arc::new(PreparedSigrokFileSourceFactory { opener })
}
