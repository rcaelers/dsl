use std::sync::Arc;

use logic_analyzer_acquisition::{CaptureMode, LogicCaptureConfig};
use logic_analyzer_capture_formats::dsl_file::{DslFileSourceConfig, DslFileSourceFactory};
use logic_analyzer_device_dslogic::DsLogicU3Pro16SourceFactory;
use logic_analyzer_graph_capabilities::node::GraphNodeCapabilityOverride;
use logic_analyzer_graph_compiler::GraphLowerer;
use logic_analyzer_graph_plan::OutputSubscriptionPlan;
use logic_analyzer_graph_runtime::{GraphRuntime, InlineSourcePreparationExecutor};
use platform_artifacts::{ArtifactRepository, SourceIdentity};
use platform_runtime::{InlineWorkExecutor, WorkExecutor};
use signal_capture::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, CaptureMetadata,
    IndexedCapturePresentation,
};
use signal_capture_session::{
    AcquisitionContext, AcquisitionResult, CaptureDataDelivery, CaptureSourceCacheIdentity,
    CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata, CaptureSourcePresentation,
    CaptureSourceRuntimeCapabilities, CaptureStartMode, ConfiguredAcquisition, PreparedAcquisition,
};
use signal_runtime::{CooperativeAppManagerFactory, ProcessNodeConstruction};

pub(crate) struct GraphHarness {
    lowerer: GraphLowerer,
    runtime: GraphRuntime,
}

impl GraphHarness {
    pub(crate) fn new() -> Self {
        Self::with_capability_overrides(Vec::new())
    }

    fn with_capability_overrides(capability_overrides: Vec<GraphNodeCapabilityOverride>) -> Self {
        Self {
            lowerer: GraphLowerer::with_capability_overrides(capability_overrides),
            runtime: GraphRuntime::with_execution(
                Box::new(InlineSourcePreparationExecutor),
                Arc::new(CooperativeAppManagerFactory),
                Arc::new(InlineWorkExecutor),
            ),
        }
    }

    pub(crate) fn lowerer(&self) -> &GraphLowerer {
        &self.lowerer
    }

    pub(crate) fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan) {
        self.lowerer.set_output_subscriptions(subscriptions);
    }
}

impl std::ops::Deref for GraphHarness {
    type Target = GraphRuntime;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}

impl std::ops::DerefMut for GraphHarness {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.runtime
    }
}

struct TestLiveSourceFactory;

struct TestLiveSourceMetadata {
    config: LogicCaptureConfig,
}

struct TestConfiguredAcquisition {
    delivery: CaptureDataDelivery,
}

impl ConfiguredAcquisition for TestConfiguredAcquisition {
    fn data_delivery(&self) -> CaptureDataDelivery {
        self.delivery
    }

    fn capture_window_samples(&self) -> u64 {
        1
    }

    fn prepare(
        self: Box<Self>,
        _context: AcquisitionContext,
        _mode: CaptureStartMode,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        unreachable!("source-override integration does not prepare capture hardware")
    }
}

impl CaptureSourceMetadata for TestLiveSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true)
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        Ok(Some(CaptureSourcePresentation::Channels(
            enabled_channels(&self.config)
                .enumerate()
                .map(|(viewer_channel, channel)| (viewer_channel, format!("Ch {channel}")))
                .collect(),
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::NotCapture
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        Ok(Some(
            enabled_channels(&self.config)
                .map(|channel| format!("Ch {channel}"))
                .collect(),
        ))
    }

    fn runtime_capabilities(&self) -> CaptureSourceRuntimeCapabilities {
        CaptureSourceRuntimeCapabilities::new(true)
    }

    fn configured_acquisition(&self) -> Result<Option<Box<dyn ConfiguredAcquisition>>, String> {
        Ok(Some(Box::new(TestConfiguredAcquisition {
            delivery: match self.config.mode {
                CaptureMode::Streaming => CaptureDataDelivery::DuringAcquisition,
                CaptureMode::Finite => CaptureDataDelivery::BufferedUpload,
            },
        })))
    }
}

fn enabled_channels(config: &LogicCaptureConfig) -> impl Iterator<Item = usize> + '_ {
    (0..u64::BITS as usize).filter(|channel| config.input_mask & (1_u64 << channel) != 0)
}

impl DsLogicU3Pro16SourceFactory for TestLiveSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true)
    }

    fn metadata(&self, config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(TestLiveSourceMetadata { config })
    }

    fn create(
        &self,
        _name: &str,
        _config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        Err("source-override integration does not construct capture hardware".into())
    }
}

struct TestDslSourceFactory;

struct TestDslSourceMetadata {
    config: DslFileSourceConfig,
}

struct UnopenedCaptureIndexFactory;

impl CaptureIndexFactory for UnopenedCaptureIndexFactory {
    fn display_name(&self) -> String {
        "test capture".into()
    }

    fn metadata(&self) -> signal_capture::Result<CaptureMetadata> {
        Err(signal_capture::Error::ParseError(
            "test presentation does not open a capture".into(),
        ))
    }

    fn open(
        self: Box<Self>,
        _artifact_repository: Arc<dyn ArtifactRepository>,
        _work_executor: Arc<dyn WorkExecutor>,
        _progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> signal_capture::Result<Box<dyn CaptureIndex + Send>> {
        Err(signal_capture::Error::ParseError(
            "test presentation does not open a capture".into(),
        ))
    }
}

impl TestDslSourceMetadata {
    fn identity(&self) -> SourceIdentity {
        SourceIdentity::from_bytes(
            *blake3::hash(self.config.path().to_string_lossy().as_bytes()).as_bytes(),
        )
    }
}

impl CaptureSourceMetadata for TestDslSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        Ok(Some(CaptureSourcePresentation::Indexed(
            IndexedCapturePresentation {
                identity: self.identity(),
                factory: Box::new(UnopenedCaptureIndexFactory),
            },
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::Stable(*self.identity().as_bytes())
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        Ok(Some(self.config.channel_names().to_vec()))
    }
}

impl DslFileSourceFactory for TestDslSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(TestDslSourceMetadata { config })
    }

    fn create(
        &self,
        _name: &str,
        _config: DslFileSourceConfig,
        _artifact_repository: Arc<dyn ArtifactRepository>,
        _work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        Err("presentation integration does not execute a file source".into())
    }
}

pub(crate) fn test_platform_compiler() -> GraphHarness {
    GraphHarness::with_capability_overrides(vec![
        logic_analyzer_graph_nodes::u3pro16_capability_override(Arc::new(TestLiveSourceFactory)),
        logic_analyzer_graph_nodes::dsl_file_source_capability_override(Arc::new(
            TestDslSourceFactory,
        )),
    ])
}

pub(crate) fn test_live_compiler(subscriptions: OutputSubscriptionPlan) -> GraphHarness {
    let mut compiler = test_platform_compiler();
    compiler.set_output_subscriptions(subscriptions);
    compiler
}
