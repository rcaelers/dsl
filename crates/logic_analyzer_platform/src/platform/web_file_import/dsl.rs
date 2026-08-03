use std::sync::Arc;

use logic_analyzer_processing::nodes::sources::dsl_file::{
    DslFileSource, DslFileSourceConfig, DslFileSourceFactory,
};
use logic_analyzer_processing::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};
use signal_processing::{
    ArtifactRepository, CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory,
    IndexedCapturePresentation, WorkExecutor,
};

use super::registry::{BrowserFileRegistry, ImportedFile};
use super::worker_source::dsl_preparation_request;

const FILE_SOURCE_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct BrowserDslCaptureIndexFactory {
    imported: ImportedFile,
}

impl CaptureIndexFactory for BrowserDslCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.imported.display_name.clone()
    }

    fn preparation_request(&self) -> Option<signal_processing::CaptureIndexPreparationRequest> {
        self.imported
            .worker_reference
            .as_ref()
            .map(dsl_preparation_request)
    }

    fn metadata(&self) -> signal_processing::Result<signal_processing::CaptureMetadata> {
        if let Some(metadata) = &self.imported.metadata {
            return Ok(metadata.clone());
        }
        let source = self
            .imported
            .source
            .as_ref()
            .expect("resident browser captures retain their prepared source");
        DslFileSource::indexed_capture_presentation(
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
    ) -> signal_processing::Result<Box<dyn CaptureIndex + Send>> {
        let source = self
            .imported
            .source
            .expect("local capture preparation requires resident browser bytes");
        DslFileSource::indexed_capture_presentation(source, self.imported.display_name)
            .factory
            .open(artifact_repository, work_executor, progress)
    }
}

struct BrowserDslFileSourceMetadata {
    config: DslFileSourceConfig,
    registry: Arc<BrowserFileRegistry>,
}

impl CaptureSourceMetadata for BrowserDslFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        let imported = self.registry.resolve(self.config.path())?;
        Ok(Some(CaptureSourcePresentation::Indexed(
            IndexedCapturePresentation {
                identity: imported.identity,
                factory: Box::new(BrowserDslCaptureIndexFactory { imported }),
            },
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        self.registry
            .resolve(self.config.path())
            .map(|imported| CaptureSourceCacheIdentity::Stable(*imported.identity.as_bytes()))
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        let imported = self.registry.resolve(self.config.path())?;
        if let Some(metadata) = imported.metadata {
            return Ok(Some(metadata.probe_names));
        }
        DslFileSource::from_prepared_source(
            imported
                .source
                .expect("resident browser captures retain their prepared source"),
            imported.display_name,
        )
        .map_err(|error| error.to_string())
        .map(|source| Some(source.header().probe_names.clone()))
    }
}

struct BrowserDslFileSourceFactory {
    registry: Arc<BrowserFileRegistry>,
    capture_worker: Option<Arc<signal_processing::CaptureWorkerClient>>,
}

impl DslFileSourceFactory for BrowserDslFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(BrowserDslFileSourceMetadata {
            config,
            registry: Arc::clone(&self.registry),
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
        let imported = self.registry.resolve(config.path())?;
        if let Some(source) = imported.source {
            return DslFileSource::from_prepared_source(source, imported.display_name)
                .map_err(|error| error.to_string())
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
                });
        }
        let client = self.capture_worker.clone().ok_or_else(|| {
            "worker-owned browser capture replay requires a capture worker".to_owned()
        })?;
        let request = imported
            .worker_reference
            .as_ref()
            .map(dsl_preparation_request)
            .ok_or_else(|| "worker-owned browser capture has no preparation request".to_owned())?;
        let capture_metadata = imported
            .metadata
            .ok_or_else(|| "worker-owned browser capture has no metadata".to_owned())?;
        Ok(ProcessNodeConstruction::new(
            Box::new(signal_processing::CaptureWorkerReplaySource::new(
                name,
                client,
                request,
                capture_metadata,
            )),
            metadata,
        ))
    }
}

pub(crate) fn dsl_source_factory(
    registry: Arc<BrowserFileRegistry>,
    capture_worker: Option<Arc<signal_processing::CaptureWorkerClient>>,
) -> Arc<dyn DslFileSourceFactory> {
    Arc::new(BrowserDslFileSourceFactory {
        registry,
        capture_worker,
    })
}
