//! `Event Gate` graph-node definition.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::api::{EnumValue, InputDef, NodeDef, OutputDef, PanelSection, PropDef};

use crate::sockets::{COLOR_LOGIC, Signal, Trigger};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EventGateState {
    pub(crate) polarity: EnumValue,
}

pub(crate) struct EventGate;

impl NodeDef for EventGate {
    type State = EventGateState;

    fn name() -> &'static str {
        "Event Gate"
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
            InputDef::new::<Signal>("Gate").stable_id("gate"),
        ]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Trigger>("Events").stable_id("events")]
    }

    fn state() -> Self::State {
        EventGateState {
            polarity: EnumValue::new(0, &["Active high", "Active low"]),
        }
    }

    fn panels() -> Vec<node_graph::api::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Gate",
            vec![PropDef::control("polarity", "Polarity", |state| {
                &mut state.polarity
            })],
        )]
    }
}
