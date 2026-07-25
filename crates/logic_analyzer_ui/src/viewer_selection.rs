use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use logic_analyzer_graph_api::node::{
    CollectedPayloadRegistration, RuntimeBuilder, graph_node_registrations,
};
use logic_analyzer_graph_api::node_support::{PortKind, ViewerOutputControl};
use logic_analyzer_graph_compiler::OutputSubscriptionPlan;
use node_graph::{GraphState, NodeId, NodeKind};

const EXTENSION: &str = "logic_analyzer_graph.viewer_selections";
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
struct SavedEndpoint {
    node: NodeId,
    output: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct SavedSelection {
    endpoint: SavedEndpoint,
    selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct SavedSelections {
    version: u32,
    selections: Vec<SavedSelection>,
}

pub(crate) struct ViewerSelectionWarning {
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewerOutputSelection {
    pub(crate) node: NodeId,
    pub(crate) output: usize,
    pub(crate) output_id: String,
    pub(crate) label: String,
    pub(crate) selected: bool,
    pub(crate) indicator_outputs: Vec<usize>,
}

struct SelectionContracts {
    builders: HashMap<String, Box<dyn RuntimeBuilder>>,
    subscribable_kinds: HashSet<PortKind>,
}

impl SelectionContracts {
    fn from_inventory() -> Self {
        let builders = graph_node_registrations()
            .into_iter()
            .filter_map(|registration| {
                registration
                    .builder()
                    .map(|builder| (registration.name().to_owned(), builder))
            })
            .collect();
        let subscribable_kinds = inventory::iter::<CollectedPayloadRegistration>
            .into_iter()
            .map(CollectedPayloadRegistration::kind)
            .collect();
        Self {
            builders,
            subscribable_kinds,
        }
    }
}

fn saved_map(graph: &GraphState) -> HashMap<SavedEndpoint, bool> {
    graph
        .extension::<SavedSelections>(EXTENSION)
        .ok()
        .flatten()
        .into_iter()
        .flat_map(|saved| saved.selections)
        .map(|selection| (selection.endpoint, selection.selected))
        .collect()
}

pub(crate) fn viewer_output_selections(graph: &GraphState) -> Vec<ViewerOutputSelection> {
    let contracts = SelectionContracts::from_inventory();
    let saved = saved_map(graph);
    let mut selections = Vec::new();
    for (&node_id, node) in &graph.nodes {
        if node.kind != NodeKind::Regular {
            continue;
        }
        let builder = contracts.builders.get(node.def_name());
        for (output_index, output) in node.outputs.iter().enumerate() {
            if !output.visible {
                continue;
            }
            let output_id = if output.schema_id.is_empty() {
                output_index.to_string()
            } else {
                output.schema_id.clone()
            };
            let endpoint = SavedEndpoint {
                node: node_id,
                output: output_id.clone(),
            };
            let legacy = output
                .extensions
                .get("show_in_view")
                .and_then(serde_json::Value::as_bool);
            let saved_selection = saved.get(&endpoint).copied();
            let (default_selected, indicator_outputs) = if let Some(builder) = builder {
                let Some(ViewerOutputControl::Selectable {
                    default_selected,
                    indicator_outputs,
                }) = builder.viewer_output_control(output, &node.state)
                else {
                    continue;
                };
                let viewable = builder.viewer_channel_origin(output, &node.state).is_some()
                    || builder
                        .offered_kinds(output, &node.state)
                        .into_iter()
                        .any(|kind| contracts.subscribable_kinds.contains(&kind));
                if !viewable {
                    continue;
                }
                (default_selected, indicator_outputs)
            } else if saved_selection.or(legacy).is_some() {
                (false, vec![output_index])
            } else {
                continue;
            };
            selections.push(ViewerOutputSelection {
                node: node_id,
                output: output_index,
                output_id,
                label: output.name.clone(),
                selected: saved_selection.or(legacy).unwrap_or(default_selected),
                indicator_outputs,
            });
        }
    }
    selections.sort_by_key(|selection| (selection.node.0, selection.output));
    selections
}

pub(crate) fn output_subscription_plan(graph: &GraphState) -> OutputSubscriptionPlan {
    viewer_output_selections(graph)
        .into_iter()
        .filter(|selection| selection.selected)
        .map(|selection| (selection.node, selection.output))
        .collect()
}

pub(crate) fn synchronize_viewer_selections(
    graph: &mut GraphState,
) -> Result<Vec<ViewerSelectionWarning>, serde_json::Error> {
    let saved = match graph.extension::<SavedSelections>(EXTENSION) {
        Ok(saved) => saved,
        Err(error) => {
            return Ok(vec![ViewerSelectionWarning {
                message: format!(
                    "Could not read the saved viewer-selection manifest; it was preserved unchanged: {error}"
                ),
            }]);
        }
    };
    if let Some(saved) = saved
        && saved.version != VERSION
    {
        return Ok(vec![ViewerSelectionWarning {
            message: format!(
                "Viewer-selection manifest version {} is not supported by this version; it was preserved unchanged",
                saved.version
            ),
        }]);
    }
    let had_legacy = graph.nodes.values().any(|node| {
        node.outputs
            .iter()
            .any(|output| output.extensions.contains_key("show_in_view"))
    });
    let selections = viewer_output_selections(graph);
    for node in graph.nodes.values_mut() {
        for output in &mut node.outputs {
            output.extensions.remove("show_in_view");
        }
    }
    store(graph, &selections)?;
    Ok(had_legacy
        .then(|| ViewerSelectionWarning {
            message: "Migrated legacy socket viewer selections to the LogicConduit viewer-selection manifest"
                .to_owned(),
        })
        .into_iter()
        .collect())
}

pub(crate) fn set_viewer_output_selected(
    graph: &mut GraphState,
    node: NodeId,
    output_id: &str,
    selected: bool,
) -> Result<(), serde_json::Error> {
    let mut selections = viewer_output_selections(graph);
    if let Some(selection) = selections
        .iter_mut()
        .find(|selection| selection.node == node && selection.output_id == output_id)
    {
        selection.selected = selected;
    }
    store(graph, &selections)
}

fn store(
    graph: &mut GraphState,
    selections: &[ViewerOutputSelection],
) -> Result<(), serde_json::Error> {
    if selections.is_empty() {
        graph.remove_extension(EXTENSION);
        return Ok(());
    }
    graph.set_extension(
        EXTENSION,
        SavedSelections {
            version: VERSION,
            selections: selections
                .iter()
                .map(|selection| SavedSelection {
                    endpoint: SavedEndpoint {
                        node: selection.node,
                        output: selection.output_id.clone(),
                    },
                    selected: selection.selected,
                })
                .collect(),
        },
    )
}
