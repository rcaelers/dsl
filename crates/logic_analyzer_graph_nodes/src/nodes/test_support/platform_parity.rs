use std::sync::Arc;

use logic_analyzer_graph_api::node::RuntimeBuilder;
use logic_analyzer_processing::nodes::sinks::binary_file_writer::{
    BinaryFileWriterConfig, BinaryFileWriterFactory,
};
use logic_analyzer_processing::nodes::sinks::csv_word_writer::{
    CsvWordWriterConfig, CsvWordWriterFactory,
};
use logic_analyzer_processing::nodes::sinks::text_file_writer::TextFileWriterFactory;
use logic_analyzer_processing::nodes::sources::dsl_file::{
    DslFileSourceConfig, DslFileSourceFactory,
};
use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::DsLogicU3Pro16SourceFactory;
use logic_analyzer_processing::nodes::sources::sigrok_file::{
    SigrokFileSourceConfig, SigrokFileSourceFactory,
};
use logic_analyzer_processing::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};
use signal_processing::logic_analyzer::LogicCaptureConfig;

use super::process_node::TestProcessNode;

pub(crate) struct PlatformParityBuilderRegistration {
    stable_id: &'static str,
    create: fn() -> Box<dyn RuntimeBuilder>,
}

impl PlatformParityBuilderRegistration {
    pub(crate) const fn new(
        stable_id: &'static str,
        create: fn() -> Box<dyn RuntimeBuilder>,
    ) -> Self {
        Self { stable_id, create }
    }
}

inventory::collect!(PlatformParityBuilderRegistration);

pub(crate) fn platform_parity_builder(stable_id: &str) -> Box<dyn RuntimeBuilder> {
    inventory::iter::<PlatformParityBuilderRegistration>
        .into_iter()
        .find(|registration| registration.stable_id == stable_id)
        .map(|registration| (registration.create)())
        .unwrap_or_else(|| panic!("no platform-parity builder for '{stable_id}'"))
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

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        Ok(Some(CaptureSourcePresentation::Channels(Vec::new())))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::NotCapture
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
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
        _work_executor: Arc<dyn signal_processing::WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
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
        _work_executor: Arc<dyn signal_processing::WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        Ok(self.construction(name))
    }
}

impl DsLogicU3Pro16SourceFactory for TestSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        self.lifecycle
    }

    fn metadata(&self, _config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata> {
        self.metadata()
    }

    fn create(
        &self,
        name: &str,
        _config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        Ok(self.construction(name))
    }
}

pub(crate) struct TestWriterFactory;

impl BinaryFileWriterFactory for TestWriterFactory {
    fn create(
        &self,
        name: &str,
        _config: BinaryFileWriterConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        Ok(writer_construction(name))
    }
}

impl CsvWordWriterFactory for TestWriterFactory {
    fn create(
        &self,
        name: &str,
        _config: CsvWordWriterConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        Ok(writer_construction(name))
    }
}

impl TextFileWriterFactory for TestWriterFactory {
    fn create(&self, name: &str) -> Result<ProcessNodeConstruction, String> {
        Ok(writer_construction(name))
    }
}

fn writer_construction(name: &str) -> ProcessNodeConstruction {
    ProcessNodeConstruction::new(Box::new(TestProcessNode::new(name)), ())
}
