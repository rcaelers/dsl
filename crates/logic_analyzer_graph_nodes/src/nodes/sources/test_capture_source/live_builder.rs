//! Native deterministic live-capture builder used by tests.

use serde_json::Value;

use logic_analyzer_graph_capabilities::node::{
    CaptureSourceFeature, CaptureSourceFeatureError, GraphNodePresentation, GraphNodeSemantics,
    LiveCaptureFeature, LiveCaptureFeatureError, LiveCaptureFeatureProvider,
    RuntimeMaterializationError, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    CapturePresentation, LiveCaptureEdit, NodeBuildContext, PortKind, ResolvedInputs,
    SimpleTriggerChannel, TriggerConfigurationFeature, parse_state,
};
use logic_analyzer_trigger::{SimpleTriggerCondition, TriggerPredicate, TriggerProgram};
use node_graph_document::SocketReference;
use signal_runtime::ProcessNode;

use super::builder::TestCaptureSourceBuilder;

#[derive(Default)]
pub(crate) struct TestLiveCaptureSourceBuilder;

pub(crate) fn conditions(
    program: Option<&TriggerProgram>,
) -> Result<Vec<SimpleTriggerCondition>, String> {
    let channel_ids = super::trigger::channel_ids();
    super::trigger::validate_program(program)?;
    let mut conditions = std::collections::BTreeMap::new();
    if let Some(stage) = program.and_then(|program| program.stages.first()) {
        for predicate in &stage.predicates {
            let TriggerPredicate::Digital { channel, condition } = predicate else {
                unreachable!("validated demo schemas contain only digital predicates");
            };
            conditions.insert(channel.clone(), *condition);
        }
    }
    Ok(channel_ids
        .iter()
        .map(|channel| {
            conditions
                .get(channel)
                .copied()
                .unwrap_or(SimpleTriggerCondition::Ignore)
        })
        .collect())
}

fn configuration(
    state: &super::definition::TestCaptureSourceState,
) -> Result<TriggerConfigurationFeature, LiveCaptureFeatureError> {
    let conditions =
        conditions(state.trigger_program()).map_err(LiveCaptureFeatureError::configuration)?;
    let channels = super::trigger::channel_ids()
        .into_iter()
        .zip(conditions)
        .enumerate()
        .map(
            |(viewer_channel, (channel_id, condition))| SimpleTriggerChannel {
                channel_id,
                viewer_channel,
                name: format!("D{viewer_channel}"),
                enabled: true,
                condition,
            },
        )
        .collect();
    Ok(TriggerConfigurationFeature::new(
        super::trigger::schema(),
        state.trigger_program().cloned(),
        channels,
    )?)
}

impl GraphNodeSemantics for TestLiveCaptureSourceBuilder {
    fn is_source(&self) -> bool {
        true
    }

    fn source_data_lifecycle(
        &self,
    ) -> Option<logic_analyzer_graph_capabilities::node_support::SourceDataLifecycle> {
        Some(
            logic_analyzer_graph_capabilities::node_support::SourceDataLifecycle::new(
                logic_analyzer_graph_capabilities::node_support::SourceDataLifecycleKind::Live,
                false,
                true,
                true,
            ),
        )
    }

    fn accepted_kinds(&self, socket: SocketReference<'_>, state: &Value) -> Vec<PortKind> {
        TestCaptureSourceBuilder.accepted_kinds(socket, state)
    }

    fn offered_kinds(&self, socket: SocketReference<'_>, state: &Value) -> Vec<PortKind> {
        TestCaptureSourceBuilder.offered_kinds(socket, state)
    }

    fn input_port(
        &self,
        socket: SocketReference<'_>,
        state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        TestCaptureSourceBuilder.input_port(socket, state, kind)
    }

    fn output_port(
        &self,
        socket: SocketReference<'_>,
        state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        TestCaptureSourceBuilder.output_port(socket, state, kind)
    }

    fn input_required(&self, socket: SocketReference<'_>, state: &Value) -> bool {
        TestCaptureSourceBuilder.input_required(socket, state)
    }
}

impl CaptureSourceFeature for TestLiveCaptureSourceBuilder {
    fn capture_presentation(
        &self,
        state: &Value,
    ) -> Result<Option<CapturePresentation>, CaptureSourceFeatureError> {
        TestCaptureSourceBuilder.capture_presentation(state)
    }
}

impl GraphNodePresentation for TestLiveCaptureSourceBuilder {
    fn viewer_channel_origin(&self, socket: SocketReference<'_>, state: &Value) -> Option<usize> {
        TestCaptureSourceBuilder.viewer_channel_origin(socket, state)
    }
}

impl LiveCaptureFeatureProvider for TestLiveCaptureSourceBuilder {
    fn live_capture_feature(
        &self,
        state: &Value,
    ) -> Result<Option<Box<dyn LiveCaptureFeature>>, LiveCaptureFeatureError> {
        super::live_capture::feature(state)
    }

    fn trigger_configuration(
        &self,
        state: &Value,
    ) -> Result<Option<TriggerConfigurationFeature>, LiveCaptureFeatureError> {
        let state = parse_state::<super::definition::TestCaptureSourceState>(state)?;
        configuration(&state).map(Some)
    }

    fn apply_live_capture_edit(
        &self,
        state: &Value,
        edit: &LiveCaptureEdit,
    ) -> Result<Option<Value>, LiveCaptureFeatureError> {
        super::implementation::apply_live_capture_edit(state, edit).map(Some)
    }
}

impl RuntimeMaterializer for TestLiveCaptureSourceBuilder {
    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, RuntimeMaterializationError> {
        TestCaptureSourceBuilder.build(name, state, resolved, ctx)
    }
}
