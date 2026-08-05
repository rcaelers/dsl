//! Runtime builder for the DSLogic U3Pro16 graph source.

use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{LiveCaptureFeature, RuntimeBuilder};
use logic_analyzer_graph_capabilities::node_support::{
    CapturePresentation, LiveCaptureEdit, NodeBuildContext, PortKind, ResolvedInputs,
    TriggerConfigurationFeature, parse_state,
};
use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::{
    DsLogicU3Pro16SourceFactory, unavailable_source_factory,
};
use logic_analyzer_processing::{CaptureSourceMetadata, ProcessNodeConstruction};
use node_graph::api::Socket;
use signal_processing::{DerivedDataRetention, Sample, SampleBlock};
use signal_runtime::ProcessNode;

use super::definition::U3Pro16State;

pub(crate) struct DsLogicU3Pro16Builder {
    source_factory: Arc<dyn DsLogicU3Pro16SourceFactory>,
}

impl Default for DsLogicU3Pro16Builder {
    fn default() -> Self {
        Self {
            source_factory: unavailable_source_factory(),
        }
    }
}

impl DsLogicU3Pro16Builder {
    pub(crate) fn with_source_factory(
        source_factory: Arc<dyn DsLogicU3Pro16SourceFactory>,
    ) -> Self {
        Self { source_factory }
    }

    fn config(
        state: &Value,
    ) -> Result<signal_processing::logic_analyzer::LogicCaptureConfig, String> {
        let state: U3Pro16State = parse_state(state)?;
        super::capture_configuration::capture_config(&state)
    }

    fn metadata(&self, state: &Value) -> Result<Arc<dyn CaptureSourceMetadata>, String> {
        Ok(self.source_factory.metadata(Self::config(state)?))
    }
}

pub(crate) fn runtime_builder_override(
    source_factory: Arc<dyn DsLogicU3Pro16SourceFactory>,
) -> logic_analyzer_graph_capabilities::node::RuntimeBuilderOverride {
    logic_analyzer_graph_capabilities::node::RuntimeBuilderOverride::new(
        "org.logicconduit.graph-node.sources.dslogic-u3pro16/v1",
        Box::new(DsLogicU3Pro16Builder::with_source_factory(source_factory)),
    )
}

impl RuntimeBuilder for DsLogicU3Pro16Builder {
    fn is_source(&self) -> bool {
        true
    }

    fn source_data_lifecycle(
        &self,
    ) -> Option<logic_analyzer_graph_capabilities::node_support::SourceDataLifecycle> {
        Some(super::super::metadata::lifecycle(
            self.source_factory.lifecycle(),
        ))
    }

    fn derived_data_retention(&self, _state: &Value) -> DerivedDataRetention {
        DerivedDataRetention::Unlimited
    }

    fn accepted_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        Vec::new()
    }

    fn offered_kinds(&self, _socket: &Socket, _state: &Value) -> Vec<PortKind> {
        vec![PortKind::of::<SampleBlock>(), PortKind::of::<Sample>()]
    }

    fn input_port(
        &self,
        _socket: &Socket,
        _member: usize,
        _state: &Value,
        _kind: PortKind,
    ) -> Option<String> {
        None
    }

    fn output_port(&self, socket: &Socket, state: &Value, kind: PortKind) -> Option<String> {
        if kind != PortKind::of::<Sample>() && kind != PortKind::of::<SampleBlock>() {
            return None;
        }
        let state: U3Pro16State = parse_state(state).ok()?;
        if !state
            .channels
            .enabled
            .get(socket.def_index)
            .copied()
            .unwrap_or(false)
        {
            return None;
        }
        let logical_channel = state.channels.enabled[..socket.def_index]
            .iter()
            .filter(|enabled| **enabled)
            .count();
        Some(format!("ch{logical_channel}"))
    }

    fn viewer_channel_origin(&self, socket: &Socket, state: &Value) -> Option<usize> {
        let state: U3Pro16State = parse_state(state).ok()?;
        if !state
            .channels
            .enabled
            .get(socket.def_index)
            .copied()
            .unwrap_or(false)
        {
            return None;
        }
        Some(
            state.channels.enabled[..socket.def_index]
                .iter()
                .filter(|enabled| **enabled)
                .count(),
        )
    }

    fn capture_presentation(&self, state: &Value) -> Result<Option<CapturePresentation>, String> {
        self.metadata(state)?
            .presentation()
            .map(|presentation| presentation.map(super::super::metadata::presentation))
    }

    fn live_capture_feature(
        &self,
        state: &Value,
    ) -> Result<Option<Box<dyn LiveCaptureFeature>>, String> {
        let metadata = self.metadata(state)?;
        if !metadata.runtime_capabilities().live_acquisition() {
            return Ok(None);
        }
        let acquisition = metadata.configured_acquisition()?.ok_or_else(|| {
            "capture source advertises live acquisition without providing it".to_owned()
        })?;
        super::live_capture::feature(state, acquisition)
    }

    fn trigger_configuration(
        &self,
        state: &Value,
    ) -> Result<Option<TriggerConfigurationFeature>, String> {
        let state: U3Pro16State = parse_state(state)?;
        super::trigger::configuration(&state).map(Some)
    }

    fn apply_live_capture_edit(
        &self,
        state: &Value,
        edit: &LiveCaptureEdit,
    ) -> Result<Option<Value>, String> {
        super::live_edit::apply(state, edit).map(Some)
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let config = Self::config(state)?;
        self.source_factory
            .create(name, config)
            .map(ProcessNodeConstruction::into_process)
    }
}

#[cfg(test)]
fn platform_parity_builder() -> Box<dyn RuntimeBuilder> {
    Box::new(DsLogicU3Pro16Builder::with_source_factory(Arc::new(
        crate::nodes::test_support::TestSourceFactory::live(),
    )))
}

#[cfg(test)]
inventory::submit! {
    crate::nodes::test_support::PlatformParityBuilderRegistration::new(
        "org.logicconduit.graph-node.sources.dslogic-u3pro16/v1",
        platform_parity_builder,
    )
}

#[cfg(test)]
mod builder_tests {
    use std::sync::Mutex;

    use logic_analyzer_processing::{
        CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle,
        CaptureSourcePresentation,
    };
    use signal_processing::logic_analyzer::LogicCaptureConfig;
    use signal_runtime::{InputPort, OutputPort, WorkResult};

    use super::*;

    struct FakeSource {
        name: String,
    }

    impl ProcessNode for FakeSource {
        fn name(&self) -> &str {
            &self.name
        }

        fn num_inputs(&self) -> usize {
            0
        }

        fn num_outputs(&self) -> usize {
            0
        }

        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct FakeSourceFactory {
        opened: Mutex<Option<(String, LogicCaptureConfig)>>,
        error: Option<String>,
    }

    struct FakeMetadata;

    impl CaptureSourceMetadata for FakeMetadata {
        fn lifecycle(&self) -> CaptureSourceLifecycle {
            CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true)
        }

        fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
            Ok(Some(CaptureSourcePresentation::Channels(vec![(
                0,
                "Ch 0".into(),
            )])))
        }

        fn cache_identity(&self) -> CaptureSourceCacheIdentity {
            CaptureSourceCacheIdentity::NotCapture
        }

        fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
            Ok(Some(vec!["Ch 0".into()]))
        }
    }

    impl DsLogicU3Pro16SourceFactory for FakeSourceFactory {
        fn lifecycle(&self) -> CaptureSourceLifecycle {
            FakeMetadata.lifecycle()
        }

        fn metadata(&self, _config: LogicCaptureConfig) -> Arc<dyn CaptureSourceMetadata> {
            Arc::new(FakeMetadata)
        }

        fn create(
            &self,
            name: &str,
            config: LogicCaptureConfig,
        ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            *self.opened.lock().unwrap() = Some((name.to_owned(), config.clone()));
            Ok(ProcessNodeConstruction::new(
                Box::new(FakeSource {
                    name: name.to_owned(),
                }),
                self.metadata(config),
            ))
        }
    }

    #[test]
    fn runtime_lowering_uses_the_injected_source_factory() {
        let factory = Arc::new(FakeSourceFactory::default());
        let builder = DsLogicU3Pro16Builder::with_source_factory(factory.clone());
        let state = serde_json::to_value(U3Pro16State::default()).unwrap();
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        let runtime = builder
            .build(
                "Test U3Pro16",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .unwrap();

        assert_eq!(runtime.name(), "Test U3Pro16");
        let opened = factory.opened.lock().unwrap();
        let (name, config) = opened.as_ref().expect("fake source was opened");
        assert_eq!(name, "Test U3Pro16");
        assert!(config.sample_rate_hz > 0);
        assert_ne!(config.input_mask, 0);
    }

    #[test]
    fn parity_factory_preserves_enabled_channel_presentation() {
        let builder = platform_parity_builder();
        let state = serde_json::to_value(U3Pro16State::default()).unwrap();

        let presentation = builder.capture_presentation(&state).unwrap();
        let Some(CapturePresentation::Channels(channels)) = presentation else {
            panic!("expected channel presentation");
        };
        assert_eq!(channels.len(), 16);
    }

    #[test]
    fn registered_parity_factory_preserves_enabled_channel_presentation() {
        let builder = crate::nodes::test_support::platform_parity_builder(
            "org.logicconduit.graph-node.sources.dslogic-u3pro16/v1",
        );
        let state = serde_json::to_value(U3Pro16State::default()).unwrap();

        let presentation = builder.capture_presentation(&state).unwrap();
        assert!(matches!(
            presentation,
            Some(CapturePresentation::Channels(_))
        ));
    }

    #[test]
    fn runtime_lowering_reports_source_factory_errors() {
        let builder = DsLogicU3Pro16Builder::with_source_factory(Arc::new(FakeSourceFactory {
            error: Some("test device unavailable".to_owned()),
            ..FakeSourceFactory::default()
        }));
        let state = serde_json::to_value(U3Pro16State::default()).unwrap();
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        let error = builder
            .build(
                "Test U3Pro16",
                &state,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("factory error must be preserved");

        assert_eq!(error, "test device unavailable");
    }

    #[test]
    fn malformed_state_is_rejected_before_opening_a_source() {
        let factory = Arc::new(FakeSourceFactory::default());
        let builder = DsLogicU3Pro16Builder::with_source_factory(factory.clone());
        let mut context = crate::nodes::test_support::TestNodeBuildContext::default();

        let error = builder
            .build(
                "Test U3Pro16",
                &Value::Null,
                &ResolvedInputs::default(),
                &mut context,
            )
            .err()
            .expect("malformed state must fail");

        assert!(error.starts_with("invalid node state:"));
        assert!(factory.opened.lock().unwrap().is_none());
    }
}
