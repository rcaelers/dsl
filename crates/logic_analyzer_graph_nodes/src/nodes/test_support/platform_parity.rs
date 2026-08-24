use std::sync::Arc;

use logic_analyzer_acquisition::LogicCaptureConfig;
use logic_analyzer_capture_formats::CaptureSourceConstructionError;
use logic_analyzer_capture_formats::dsl_file::{DslFileSourceConfig, DslFileSourceFactory};
use logic_analyzer_capture_formats::sigrok_file::{
    SigrokFileSourceConfig, SigrokFileSourceFactory,
};
use logic_analyzer_device_dslogic::{DsLogicU3Pro16SourceError, DsLogicU3Pro16SourceFactory};
use logic_analyzer_graph_capabilities::node::{
    CaptureSourceFeature, GraphNodePresentation, GraphNodeSemantics, RuntimeMaterializer,
};
use signal_capture_session::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourceMetadataError, CaptureSourcePresentation,
};
use signal_runtime::ProcessNodeConstruction;
use signal_sinks::binary_file_writer::{BinaryFileWriterConfig, BinaryFileWriterFactory};
use signal_sinks::csv_word_writer::{CsvWordWriterConfig, CsvWordWriterFactory};
use signal_sinks::text_file_writer::TextFileWriterFactory;
use signal_sinks::{OutputOrigin, WriterConstructionError};

use super::process_node::TestProcessNode;

pub(crate) struct PlatformParityCapabilities {
    pub(crate) semantics: Box<dyn GraphNodeSemantics>,
    pub(crate) materializer: Box<dyn RuntimeMaterializer>,
    pub(crate) capture_source: Option<Box<dyn CaptureSourceFeature>>,
    pub(crate) presentation: Option<Box<dyn GraphNodePresentation>>,
}

impl PlatformParityCapabilities {
    pub(crate) fn new(
        semantics: Box<dyn GraphNodeSemantics>,
        materializer: Box<dyn RuntimeMaterializer>,
    ) -> Self {
        Self {
            semantics,
            materializer,
            capture_source: None,
            presentation: None,
        }
    }

    pub(crate) fn with_capture_source(
        mut self,
        capture_source: Box<dyn CaptureSourceFeature>,
    ) -> Self {
        self.capture_source = Some(capture_source);
        self
    }

    pub(crate) fn with_presentation(
        mut self,
        presentation: Box<dyn GraphNodePresentation>,
    ) -> Self {
        self.presentation = Some(presentation);
        self
    }
}

pub(crate) struct PlatformParityCapabilityRegistration {
    stable_id: &'static str,
    create: fn() -> PlatformParityCapabilities,
}

impl PlatformParityCapabilityRegistration {
    pub(crate) const fn new(
        stable_id: &'static str,
        create: fn() -> PlatformParityCapabilities,
    ) -> Self {
        Self { stable_id, create }
    }
}

inventory::collect!(PlatformParityCapabilityRegistration);

pub(crate) fn platform_parity_capabilities(stable_id: &str) -> PlatformParityCapabilities {
    inventory::iter::<PlatformParityCapabilityRegistration>
        .into_iter()
        .find(|registration| registration.stable_id == stable_id)
        .map(|registration| (registration.create)())
        .unwrap_or_else(|| panic!("no platform-parity capabilities for '{stable_id}'"))
}

pub(crate) struct TestSourceFactory {
    lifecycle: CaptureSourceLifecycle,
}

impl TestSourceFactory {
    pub(crate) fn file() -> Self {
        Self {
            lifecycle: CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true),
        }
    }

    pub(crate) fn live() -> Self {
        Self {
            lifecycle: CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true),
        }
    }

    fn metadata(&self) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(TestMetadata {
            lifecycle: self.lifecycle,
        })
    }

    fn construction(&self, name: &str) -> ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>> {
        ProcessNodeConstruction::new(Box::new(TestProcessNode::new(name)), self.metadata())
    }
}

struct TestMetadata {
    lifecycle: CaptureSourceLifecycle,
}

impl CaptureSourceMetadata for TestMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        self.lifecycle
    }

    fn presentation(
        &self,
    ) -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError> {
        Ok(Some(CaptureSourcePresentation::Channels(Vec::new())))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::NotCapture
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, CaptureSourceMetadataError> {
        Ok(Some(Vec::new()))
    }
}

impl DslFileSourceFactory for TestSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        self.lifecycle
    }

    fn metadata(&self, _config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        self.metadata()
    }

    fn create(
        &self,
        name: &str,
        _config: DslFileSourceConfig,
        _artifact_repository: Arc<dyn platform_artifacts::ArtifactRepository>,
        _work_executor: Arc<dyn platform_runtime::WorkExecutor>,
    ) -> Result<
        ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>,
        CaptureSourceConstructionError,
    > {
        Ok(self.construction(name))
    }
}

impl SigrokFileSourceFactory for TestSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        self.lifecycle
    }

    fn metadata(&self, _config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        self.metadata()
    }

    fn create(
        &self,
        name: &str,
        _config: SigrokFileSourceConfig,
        _work_executor: Arc<dyn platform_runtime::WorkExecutor>,
    ) -> Result<
        ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>,
        CaptureSourceConstructionError,
    > {
        Ok(self.construction(name))
    }
}

impl DsLogicU3Pro16SourceFactory for TestSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        self.lifecycle
    }

    fn metadata(&self, config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(U3Pro16TestMetadata { config })
    }

    fn create(
        &self,
        name: &str,
        _config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, DsLogicU3Pro16SourceError>
    {
        Ok(self.construction(name))
    }
}

struct U3Pro16TestMetadata {
    config: LogicCaptureConfig,
}

impl CaptureSourceMetadata for U3Pro16TestMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true)
    }

    fn presentation(
        &self,
    ) -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError> {
        Ok(Some(CaptureSourcePresentation::Channels(
            (0..u64::BITS as usize)
                .filter(|channel| self.config.input_mask & (1_u64 << channel) != 0)
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
            (0..u64::BITS as usize)
                .filter(|channel| self.config.input_mask & (1_u64 << channel) != 0)
                .map(|channel| format!("Ch {channel}"))
                .collect(),
        ))
    }
}

pub(crate) struct TestWriterFactory;

impl BinaryFileWriterFactory for TestWriterFactory {
    fn create(
        &self,
        name: &str,
        _config: BinaryFileWriterConfig,
        output_origin: OutputOrigin,
    ) -> Result<ProcessNodeConstruction, WriterConstructionError> {
        assert_writer_origin(output_origin);
        Ok(writer_construction(name))
    }
}

impl CsvWordWriterFactory for TestWriterFactory {
    fn create(
        &self,
        name: &str,
        _config: CsvWordWriterConfig,
        output_origin: OutputOrigin,
    ) -> Result<ProcessNodeConstruction, WriterConstructionError> {
        assert_writer_origin(output_origin);
        Ok(writer_construction(name))
    }
}

impl TextFileWriterFactory for TestWriterFactory {
    fn create(
        &self,
        name: &str,
        _static_filename: Option<String>,
        output_origin: OutputOrigin,
    ) -> Result<ProcessNodeConstruction, WriterConstructionError> {
        assert_writer_origin(output_origin);
        Ok(writer_construction(name))
    }
}

fn assert_writer_origin(output_origin: OutputOrigin) {
    assert_eq!(
        output_origin,
        OutputOrigin::new("Fixture source", "Output 0")
    );
}

fn writer_construction(name: &str) -> ProcessNodeConstruction {
    ProcessNodeConstruction::new(Box::new(TestProcessNode::new(name)), ())
}
