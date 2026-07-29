//! `Buffer` graph-node definition (`docs/PIPELINE_DESIGN.md`, flow control) — an explicit,
//! user-placed decoupling point. Its input and output expose the payload kind
//! selected in its state, matching the concrete runtime selected by the
//! sibling builder.

use egui::Color32;
use serde::{Deserialize, Serialize};

use node_graph::{
    EnumValue, InputDef, IntValue, NodeDef, NodeInstanceSchema, OutputDef, PanelSection, PropDef,
    SocketDef,
};

use crate::sockets::{COLOR_LOGIC, Number, Signal, Text, Trigger, Words};

/// Which built-in payload kind flows through a given `Buffer` instance —
/// order matches the dropdown and the sibling builder's
/// `selected_kind()`.
const KIND_LABELS: &[&str] = &["Signal", "Block", "Word", "Number", "Text", "Trigger"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BufferState {
    pub(crate) kind: EnumValue,
    pub(crate) capacity: IntValue,
}

pub(crate) struct Buffer;

fn passthrough_schema<T: SocketDef>() -> NodeInstanceSchema<BufferState> {
    NodeInstanceSchema::new(
        vec![InputDef::new::<T>("In")],
        vec![OutputDef::new::<T>("Out")],
    )
    .panel(Buffer::panel())
    .panels(Buffer::panels())
}

impl NodeDef for Buffer {
    type State = BufferState;

    fn name() -> &'static str {
        "Buffer"
    }
    fn category() -> &'static str {
        "Logic"
    }
    fn color() -> Color32 {
        COLOR_LOGIC
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::new::<Signal>("In")]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Signal>("Out")]
    }

    fn state() -> Self::State {
        BufferState {
            kind: EnumValue::new(0, KIND_LABELS),
            capacity: IntValue::new(1_000, 1, i32::MAX),
        }
    }

    fn panels() -> Vec<node_graph::NodePanelDef<Self::State>> {
        vec![crate::presentation::viewer_outputs_panel()]
    }

    fn instance_schema(state: &Self::State) -> NodeInstanceSchema<Self::State> {
        match state.kind.selected() {
            "Word" => passthrough_schema::<Words>(),
            "Number" => passthrough_schema::<Number>(),
            "Text" => passthrough_schema::<Text>(),
            "Trigger" => passthrough_schema::<Trigger>(),
            // Sample blocks are a transport representation of a signal and
            // therefore retain the graph-level Signal socket contract.
            _ => passthrough_schema::<Signal>(),
        }
    }

    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new(
            "Options",
            vec![
                PropDef::control("kind", "Payload", |state| &mut state.kind),
                PropDef::control("capacity", "Capacity", |state| &mut state.capacity),
            ],
        )]
    }
}

#[cfg(test)]
mod definition_tests {
    use node_graph::api::GraphDocumentBuilder;
    use node_graph::{NodeDef, NodeTypeRegistry};

    use super::Buffer;

    #[test]
    fn selected_payload_kind_is_exposed_on_both_sides() {
        let mut registry = NodeTypeRegistry::new();
        registry.register::<Buffer>();
        let mut document = GraphDocumentBuilder::new(registry);
        let node_id = document
            .add_node(Buffer::name())
            .expect("registered Buffer");
        let mut state = Buffer::state();
        state.kind.select("Word");

        assert!(document.set_node_state(node_id, serde_json::to_value(state).unwrap(),));

        let node = &document.graph().nodes[&node_id];
        assert_eq!(node.inputs[0].effective_type(), "Words");
        assert_eq!(node.outputs[0].effective_type(), "Words");
        assert_eq!(node.inputs[0].color, node.outputs[0].color);
        assert_eq!(node.inputs[0].shape, node.outputs[0].shape);
    }
}
