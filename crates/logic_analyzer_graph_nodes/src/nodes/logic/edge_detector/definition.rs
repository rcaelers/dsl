//! `Edge Detector` graph-node definition.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::api::{
    EnumValue, InputDef, IntValue, NodeBadge, NodeDef, OutputDef, PanelSection, PropDef, Socket,
};

use crate::sockets::{COLOR_LOGIC, Signal, Trigger};

const MAX_TIME_US: i32 = 2_000_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EdgeDetectorState {
    pub(crate) edge: EnumValue,
    pub(crate) debounce_us: IntValue,
    pub(crate) minimum_pulse_width_us: IntValue,
    #[serde(skip)]
    pub(crate) compatibility_warning: Option<String>,
}

pub(crate) struct EdgeDetector;

impl NodeDef for EdgeDetector {
    type State = EdgeDetectorState;

    fn name() -> &'static str {
        "Edge Detector"
    }

    fn category() -> &'static str {
        "Logic"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::new::<Signal>("Signal").stable_id("signal")]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Trigger>("Trigger").stable_id("trigger")]
    }

    fn state() -> Self::State {
        EdgeDetectorState {
            edge: EnumValue::new(0, &["Rising", "Falling", "Both"]),
            debounce_us: IntValue::new(0, 0, MAX_TIME_US),
            minimum_pulse_width_us: IntValue::new(0, 0, MAX_TIME_US),
            compatibility_warning: None,
        }
    }

    fn migrate_saved_sockets(
        state: &mut Self::State,
        _inputs: &mut Vec<Socket>,
        outputs: &mut Vec<Socket>,
    ) {
        let Some(mut legacy_index) = outputs
            .iter()
            .position(|output| output.schema_id == "events")
        else {
            return;
        };

        if let Some(current_index) = outputs
            .iter()
            .position(|output| output.schema_id == "trigger")
        {
            outputs.remove(current_index);
            if current_index < legacy_index {
                legacy_index -= 1;
            }
        }
        outputs[legacy_index].schema_id = "trigger".to_owned();
        state.compatibility_warning = Some(
            "Updated the legacy Edge Detector output identity; existing connections were preserved"
                .to_owned(),
        );
    }

    fn panels() -> Vec<node_graph::api::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Qualification",
            vec![
                PropDef::control("edge", "Edge", |state| &mut state.edge),
                PropDef::control("debounce_us", "Debounce µs", |state| {
                    &mut state.debounce_us
                }),
                PropDef::control(
                    "minimum_pulse_width_us",
                    "Minimum preceding pulse µs",
                    |state| &mut state.minimum_pulse_width_us,
                ),
            ],
        )]
    }

    fn on_update(state: &mut Self::State, _inputs: &mut [Socket], _outputs: &mut [Socket]) {
        for value in [&mut state.debounce_us, &mut state.minimum_pulse_width_us] {
            value.min = 0;
            value.max = MAX_TIME_US;
            value.value = value.value.clamp(0, MAX_TIME_US);
        }
    }

    fn badge(state: &Self::State) -> Option<NodeBadge> {
        state.compatibility_warning.as_ref().map(NodeBadge::warning)
    }
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn legacy_output_identity_replaces_a_reconciled_duplicate() {
        let mut widget = node_graph::NodeGraphWidget::new(crate::test_support::build_registry());
        let node = widget
            .add_node_at(EdgeDetector::name(), egui::Pos2::ZERO)
            .unwrap();
        let mut graph = widget.graph().clone();
        let saved = graph.nodes.get_mut(&node).unwrap();
        let current = saved.outputs[0].clone();
        saved.outputs[0].schema_id = "events".to_owned();
        saved.outputs.push(current);

        widget.set_graph(graph);

        let restored = &widget.graph().nodes[&node];
        assert_eq!(restored.outputs.len(), 1);
        assert_eq!(restored.outputs[0].schema_id, "trigger");
        assert!(
            restored
                .badge
                .as_ref()
                .is_some_and(|badge| badge.text.contains("connections were preserved"))
        );
    }
}
