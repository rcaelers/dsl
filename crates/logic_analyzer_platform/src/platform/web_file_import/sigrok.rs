use std::sync::Arc;

use logic_analyzer_processing::nodes::sources::sigrok_file::{
    SigrokFileSource, SigrokFileSourceConfig, SigrokFileSourceFactory, portable_source_factory,
};
use logic_analyzer_processing::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;
use logic_analyzer_processing::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};
use signal_processing::{
    ArtifactRepository, CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory,
    IndexedCapturePresentation, ProcessNode, WorkExecutor,
};

use super::registry::{BrowserFileRegistry, ImportedFile};

const FILE_SOURCE_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct BrowserSigrokCaptureIndexFactory {
    imported: ImportedFile,
}

impl CaptureIndexFactory for BrowserSigrokCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.imported.display_name.clone()
    }

    fn metadata(&self) -> signal_processing::Result<signal_processing::CaptureMetadata> {
        SigrokFileSource::indexed_capture_presentation(
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
        SigrokFileSource::indexed_capture_presentation(
            self.imported.source,
            self.imported.display_name,
        )
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

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.demo_data() {
            return portable_source_factory()
                .metadata(self.config.clone())
                .presentation();
        }
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        let imported = self.registry.resolve(self.config.path())?;
        Ok(Some(CaptureSourcePresentation::Indexed(
            IndexedCapturePresentation {
                identity: imported.source.identity(),
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
            .map(|imported| {
                CaptureSourceCacheIdentity::Stable(*imported.source.identity().as_bytes())
            })
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        if self.config.demo_data() {
            return Ok(Some(self.config.channel_names().to_vec()));
        }
        let imported = self.registry.resolve(self.config.path())?;
        SigrokFileSource::from_prepared_source(imported.source)
            .map_err(|error| error.to_string())
            .map(|source| Some(source.header().probe_names.clone()))
    }
}

struct BrowserSigrokFileSourceFactory {
    registry: Arc<BrowserFileRegistry>,
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
            Box::new(
                SigrokFileSource::from_prepared_source(imported.source)
                    .map_err(|error| error.to_string())?
                    .with_name(name)
                    .with_work_executor(work_executor),
            )
        };
        Ok(ProcessNodeConstruction::new(process, metadata))
    }
}

pub(crate) fn sigrok_source_factory(
    registry: Arc<BrowserFileRegistry>,
) -> Arc<dyn SigrokFileSourceFactory> {
    Arc::new(BrowserSigrokFileSourceFactory { registry })
}
