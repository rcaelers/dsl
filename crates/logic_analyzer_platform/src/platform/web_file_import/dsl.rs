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

const FILE_SOURCE_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct BrowserDslCaptureIndexFactory {
    imported: ImportedFile,
}

impl CaptureIndexFactory for BrowserDslCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.imported.display_name.clone()
    }

    fn metadata(&self) -> signal_processing::Result<signal_processing::CaptureMetadata> {
        DslFileSource::indexed_capture_presentation(
            Arc::clone(&self.imported.source),
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
        DslFileSource::indexed_capture_presentation(
            self.imported.source,
            self.imported.display_name,
        )
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
                identity: imported.source.identity(),
                factory: Box::new(BrowserDslCaptureIndexFactory { imported }),
            },
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        self.registry
            .resolve(self.config.path())
            .map(|imported| {
                CaptureSourceCacheIdentity::Stable(*imported.source.identity().as_bytes())
            })
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        let imported = self.registry.resolve(self.config.path())?;
        DslFileSource::from_prepared_source(imported.source, imported.display_name)
            .map_err(|error| error.to_string())
            .map(|source| Some(source.header().probe_names.clone()))
    }
}

struct BrowserDslFileSourceFactory {
    registry: Arc<BrowserFileRegistry>,
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
        DslFileSource::from_prepared_source(imported.source, imported.display_name)
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
            })
    }
}

pub(crate) fn dsl_source_factory(
    registry: Arc<BrowserFileRegistry>,
) -> Arc<dyn DslFileSourceFactory> {
    Arc::new(BrowserDslFileSourceFactory { registry })
}
