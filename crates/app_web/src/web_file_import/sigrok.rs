use std::sync::Arc;

use logic_analyzer_capture_formats::sigrok_file::{
    SigrokFileSource, SigrokFileSourceConfig, SigrokFileSourceFactory, portable_source_factory,
};
use platform_artifacts::ArtifactRepository;
use platform_runtime::WorkExecutor;
use signal_capture::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, IndexedCapturePresentation,
};
use signal_capture_session::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourceMetadataError, CaptureSourcePresentation,
};
use signal_generators::synthetic_capture_source::SyntheticCaptureSource;
use signal_runtime::{ProcessNode, ProcessNodeConstruction};

use super::registry::{BrowserFileRegistry, ImportedFile};
use super::worker_source::sigrok_preparation_request;

const FILE_SOURCE_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct BrowserSigrokCaptureIndexFactory {
    imported: ImportedFile,
}

impl CaptureIndexFactory for BrowserSigrokCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.imported.display_name.clone()
    }

    fn preparation_request(&self) -> Option<signal_capture::CaptureIndexPreparationRequest> {
        self.imported
            .worker_reference
            .as_ref()
            .map(sigrok_preparation_request)
    }

    fn metadata(&self) -> signal_capture::Result<signal_capture::CaptureMetadata> {
        if let Some(metadata) = &self.imported.metadata {
            return Ok(metadata.clone());
        }
        let source = self
            .imported
            .source
            .as_ref()
            .expect("resident browser captures retain their prepared source");
        SigrokFileSource::indexed_capture_presentation(
            Arc::clone(source),
            self.imported.display_name.clone(),
        )
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
            .imported
            .source
            .expect("local capture preparation requires resident browser bytes");
        SigrokFileSource::indexed_capture_presentation(source, self.imported.display_name)
            .factory
            .open(artifact_repository, work_executor, progress)
    }
}

struct BrowserSigrokFileSourceMetadata {
    config: SigrokFileSourceConfig,
    registry: Arc<BrowserFileRegistry>,
}

impl CaptureSourceMetadata for BrowserSigrokFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn presentation(
        &self,
    ) -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError> {
        if self.config.demo_data() {
            return portable_source_factory()
                .metadata(self.config.clone())
                .presentation();
        }
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        let imported = self
            .registry
            .resolve(self.config.path())
            .map_err(CaptureSourceMetadataError::access_message)?;
        Ok(Some(CaptureSourcePresentation::Indexed(
            IndexedCapturePresentation {
                identity: imported.identity,
                factory: Box::new(BrowserSigrokCaptureIndexFactory { imported }),
            },
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.demo_data() {
            return CaptureSourceCacheIdentity::NotCapture;
        }
        self.registry
            .resolve(self.config.path())
            .map(|imported| CaptureSourceCacheIdentity::Stable(*imported.identity.as_bytes()))
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, CaptureSourceMetadataError> {
        if self.config.demo_data() {
            return Ok(Some(self.config.channel_names().to_vec()));
        }
        let imported = self
            .registry
            .resolve(self.config.path())
            .map_err(CaptureSourceMetadataError::access_message)?;
        if let Some(metadata) = imported.metadata {
            return Ok(Some(metadata.probe_names));
        }
        SigrokFileSource::from_prepared_source(
            imported
                .source
                .expect("resident browser captures retain their prepared source"),
        )
        .map_err(CaptureSourceMetadataError::decode)
        .map(|source| Some(source.header().probe_names.clone()))
    }
}

struct BrowserSigrokFileSourceFactory {
    registry: Arc<BrowserFileRegistry>,
    capture_worker: Option<Arc<signal_capture::CaptureWorkerClient>>,
}

impl SigrokFileSourceFactory for BrowserSigrokFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(BrowserSigrokFileSourceMetadata {
            config,
            registry: Arc::clone(&self.registry),
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
            let imported = self.registry.resolve(config.path())?;
            if let Some(source) = imported.source {
                Box::new(
                    SigrokFileSource::from_prepared_source(source)
                        .map_err(|error| error.to_string())?
                        .with_name(name)
                        .with_work_executor(work_executor),
                ) as Box<dyn ProcessNode>
            } else {
                let client = self.capture_worker.clone().ok_or_else(|| {
                    "worker-owned browser capture replay requires a capture worker".to_owned()
                })?;
                let request = imported
                    .worker_reference
                    .as_ref()
                    .map(sigrok_preparation_request)
                    .ok_or_else(|| {
                        "worker-owned browser capture has no preparation request".to_owned()
                    })?;
                let capture_metadata = imported
                    .metadata
                    .ok_or_else(|| "worker-owned browser capture has no metadata".to_owned())?;
                Box::new(signal_capture::CaptureWorkerReplaySource::new(
                    name,
                    client,
                    request,
                    capture_metadata,
                )) as Box<dyn ProcessNode>
            }
        };
        Ok(ProcessNodeConstruction::new(process, metadata))
    }
}

pub(crate) fn sigrok_source_factory(
    registry: Arc<BrowserFileRegistry>,
    capture_worker: Option<Arc<signal_capture::CaptureWorkerClient>>,
) -> Arc<dyn SigrokFileSourceFactory> {
    Arc::new(BrowserSigrokFileSourceFactory {
        registry,
        capture_worker,
    })
}
