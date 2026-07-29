//! `Edge Detector` graph-node definition.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::{
    EnumValue, InputDef, IntValue, NodeDef, OutputDef, PanelSection, PropDef, Socket,
};

use crate::sockets::{COLOR_LOGIC, Signal, Trigger};

const MAX_TIME_US: i32 = 2_000_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EdgeDetectorState {
    pub(crate) edge: EnumValue,
    pub(crate) debounce_us: IntValue,
    pub(crate) minimum_pulse_width_us: IntValue,
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
        }
    }

    fn panels() -> Vec<node_graph::NodePanelDef<Self::State>> {
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
}
