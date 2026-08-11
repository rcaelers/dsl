//! `Event Control` graph-node definition.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::api::{InputDef, IntValue, NodeDef, OutputDef, PanelSection, PropDef, Socket};

use crate::sockets::{COLOR_LOGIC, Trigger};

const MAX_TIME_US: i32 = 2_000_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EventControlState {
    pub(crate) delay_us: IntValue,
    pub(crate) holdoff_us: IntValue,
}

pub(crate) struct EventControl;

impl NodeDef for EventControl {
    type State = EventControlState;

    fn name() -> &'static str {
        "Event Control"
    }

    fn category() -> &'static str {
        "Logic"
    }

    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![
            InputDef::new::<Trigger>("Events").stable_id("events"),
            InputDef::new::<Trigger>("Rearm").stable_id("rearm"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Trigger>("Events").stable_id("events")]
    }

    fn state() -> Self::State {
        EventControlState {
            delay_us: IntValue::new(0, 0, MAX_TIME_US),
            holdoff_us: IntValue::new(0, 0, MAX_TIME_US),
        }
    }

    fn panels() -> Vec<node_graph::api::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Timing",
            vec![
                PropDef::control("delay_us", "Delay µs", |state| &mut state.delay_us),
                PropDef::control("holdoff_us", "Holdoff µs", |state| &mut state.holdoff_us),
            ],
        )]
    }

    fn on_update(state: &mut Self::State, _inputs: &mut [Socket], _outputs: &mut [Socket]) {
        for value in [&mut state.delay_us, &mut state.holdoff_us] {
            value.min = 0;
            value.max = MAX_TIME_US;
            value.value = value.value.clamp(0, MAX_TIME_US);
        }
    }
}

#[cfg(test)]
mod definition_tests {
    use node_graph::api::NodeDef;

    use super::EventControl;

    #[test]
    fn rearm_socket_is_always_visible() {
        let inputs = EventControl::inputs();
        assert_eq!(inputs.len(), 2);
    }
}
