//! Native runtime builder for the DSLogic U3Pro16 graph source.

use std::sync::Arc;

use serde_json::Value;

use logic_analyzer_graph_api::node::{LiveCaptureFeature, RuntimeBuilder};
use logic_analyzer_graph_api::node_support::{
    CapturePresentation, LiveCaptureEdit, NodeBuildContext, PortKind, ResolvedInputs,
    TriggerConfigurationFeature, parse_state,
};
use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::DsLogicU3Pro16Source;
use node_graph::api::Socket;
use signal_processing::logic_analyzer::LogicCaptureConfig;
use signal_processing::{DerivedDataRetention, ProcessNode, Sample, SampleBlock};

use super::definition::U3Pro16State;

trait DsLogicSourceFactory: Send + Sync {
    fn open(&self, name: &str, config: LogicCaptureConfig) -> Result<Box<dyn ProcessNode>, String>;
}

struct HardwareDsLogicSourceFactory;

impl DsLogicSourceFactory for HardwareDsLogicSourceFactory {
    fn open(&self, name: &str, config: LogicCaptureConfig) -> Result<Box<dyn ProcessNode>, String> {
        let source = DsLogicU3Pro16Source::open_first(config)
            .map_err(|error| error.to_string())?
            .with_name(name);
        Ok(Box::new(source))
    }
}

pub(crate) struct DsLogicU3Pro16Builder {
    source_factory: Arc<dyn DsLogicSourceFactory>,
}

impl Default for DsLogicU3Pro16Builder {
    fn default() -> Self {
        Self {
            source_factory: Arc::new(HardwareDsLogicSourceFactory),
        }
    }
}

#[cfg(test)]
impl DsLogicU3Pro16Builder {
    fn with_source_factory(source_factory: Arc<dyn DsLogicSourceFactory>) -> Self {
        Self { source_factory }
    }
}

impl RuntimeBuilder for DsLogicU3Pro16Builder {
    fn is_source(&self) -> bool {
        true
    }

    fn source_data_lifecycle(
        &self,
    ) -> Option<logic_analyzer_graph_api::node_support::SourceDataLifecycle> {
        Some(
            logic_analyzer_graph_api::node_support::SourceDataLifecycle::new(
                logic_analyzer_graph_api::node_support::SourceDataLifecycleKind::Live,
                false,
                true,
                true,
            ),
        )
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
        let state: U3Pro16State = parse_state(state)?;
        let channels = state
            .channels
            .enabled
            .iter()
            .enumerate()
            .filter(|(_, enabled)| **enabled)
            .enumerate()
            .map(|(viewer_channel, (physical_channel, _))| {
                (viewer_channel, format!("Ch {physical_channel}"))
            })
            .collect();
        Ok(Some(CapturePresentation::Channels(channels)))
    }

    fn live_capture_feature(
        &self,
        state: &Value,
    ) -> Result<Option<Box<dyn LiveCaptureFeature>>, String> {
        super::live_capture::feature(state)
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
        super::implementation::apply_live_capture_edit(state, edit).map(Some)
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let state: U3Pro16State = parse_state(state)?;
        let config = super::implementation::capture_config(&state)?;
        self.source_factory.open(name, config)
    }
}

#[cfg(test)]
mod builder_tests {
    use std::sync::Mutex;

    use signal_processing::{InputPort, OutputPort, WorkResult};

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

    impl DsLogicSourceFactory for FakeSourceFactory {
        fn open(
            &self,
            name: &str,
            config: LogicCaptureConfig,
        ) -> Result<Box<dyn ProcessNode>, String> {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            *self.opened.lock().unwrap() = Some((name.to_owned(), config));
            Ok(Box::new(FakeSource {
                name: name.to_owned(),
            }))
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
