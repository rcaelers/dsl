//! Headless construction of graph documents from registered node definitions.

use egui::Pos2;

use crate::model::{GraphState, Node, NodeId};
use crate::runtime::NodeTypeRegistry;

/// Builds and updates a graph document without constructing the graph widget.
///
/// Node definitions still own initial state, socket schemas, migrations, and
/// dynamic schema updates. The builder retains no widget runtime or interaction
/// state, making it suitable for application services, importers, and tests
/// that operate only on [`GraphState`].
pub struct GraphDocumentBuilder {
    graph: GraphState,
    node_types: NodeTypeRegistry,
}

impl GraphDocumentBuilder {
    pub fn new(node_types: NodeTypeRegistry) -> Self {
        Self {
            graph: GraphState::default(),
            node_types,
        }
    }

    pub fn graph(&self) -> &GraphState {
        &self.graph
    }

    pub fn graph_mut(&mut self) -> &mut GraphState {
        &mut self.graph
    }

    pub fn add_node(&mut self, type_name: &str) -> Option<NodeId> {
        let id = self.graph.next_id();
        let node = if type_name == "Reroute" {
            Node::new_reroute(id, Pos2::ZERO)
        } else {
            self.node_types.instantiate(type_name, id, Pos2::ZERO)?.node
        };
        self.graph.add_node(node);
        Some(id)
    }

    /// Replaces saved state and lets the registered definition migrate it and
    /// rebuild any state-dependent sockets.
    pub fn set_node_state(&mut self, id: NodeId, state: serde_json::Value) -> bool {
        let Some(node) = self.graph.nodes.get_mut(&id) else {
            return false;
        };
        node.state = state;
        self.node_types.restore_node(node).is_some()
    }
}

#[cfg(test)]
mod document_tests {
    use serde::{Deserialize, Serialize};

    use super::super::builtins::AnySocket;
    use super::super::node::{InputDef, NodeDef, NodeInstanceSchema, OutputDef};
    use super::*;

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    struct DynamicState {
        expanded: bool,
    }

    struct DynamicDefinition;

    impl NodeDef for DynamicDefinition {
        type State = DynamicState;

        fn name() -> &'static str {
            "Dynamic"
        }

        fn category() -> &'static str {
            "Tests"
        }

        fn inputs() -> Vec<InputDef<Self::State>> {
            vec![InputDef::new::<AnySocket>("input")]
        }

        fn outputs() -> Vec<OutputDef<Self::State>> {
            vec![OutputDef::new::<AnySocket>("primary")]
        }

        fn state() -> Self::State {
            DynamicState::default()
        }

        fn instance_schema(state: &Self::State) -> NodeInstanceSchema<Self::State> {
            let mut outputs = Self::outputs();
            if state.expanded {
                outputs.push(OutputDef::new::<AnySocket>("secondary"));
            }
            NodeInstanceSchema::new(Self::inputs(), outputs)
        }
    }

    #[test]
    fn headless_builder_applies_definition_owned_dynamic_schemas() {
        let mut node_types = NodeTypeRegistry::new();
        node_types.register::<DynamicDefinition>();
        let mut document = GraphDocumentBuilder::new(node_types);
        let node = document.add_node("Dynamic").unwrap();
        assert_eq!(document.graph().nodes[&node].outputs.len(), 1);

        assert!(document.set_node_state(node, serde_json::json!({ "expanded": true })));

        assert_eq!(document.graph().nodes[&node].outputs.len(), 2);
        assert_eq!(document.graph().nodes[&node].outputs[1].name, "secondary");
    }
}
