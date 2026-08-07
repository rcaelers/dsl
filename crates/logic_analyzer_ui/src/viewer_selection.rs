//! Saved viewer-output selection, payload-subscription, and compatibility ownership.
//!
//! **Owned data and invariants.** This module owns the versioned graph extensions and migrations
//! that turn explicit output and payload selections into `OutputSubscriptionPlan`.
//!
//! **Facade.** Application and headless orchestration use its crate-private selection,
//! synchronization, planning, edit, and warning contracts.
//!
//! **Permitted owner dependencies.** The module consumes graph documents, registered generic graph
//! capabilities, and graph-plan contracts.
//!
//! **Explicit exclusions.** It does not render the viewer, execute the graph, interpret protocol
//! payloads, or infer behavior from display names.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use logic_analyzer_graph_capabilities::node_support::{PortKind, ViewerOutputControl};
use logic_analyzer_graph_plan::OutputSubscriptionPlan;
use logic_analyzer_graph_registry::{
    GraphRegistry, graph_node_registrations, payload_registrations,
};
use node_graph::{GraphState, NodeId, NodeKind, SocketDirection, SocketId};

const EXTENSION: &str = "logic_analyzer_graph.viewer_selections";
const VERSION: u32 = 1;
const PAYLOAD_EXTENSION: &str = "logic_analyzer_graph.payload_subscriptions";
const PAYLOAD_VERSION: u32 = 1;
pub(crate) const LEGACY_VIEWER_NODE_ID: &str = "org.logicconduit.graph-node.sinks.viewer/v1";

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
    pub(crate) node: Option<NodeId>,
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
enum SavedPayloadTarget {
    ShowInView { node: NodeId, output: usize },
    ViewerInput { node: NodeId, input: usize },
}

impl SavedPayloadTarget {
    fn node(&self) -> NodeId {
        match self {
            Self::ShowInView { node, .. } | Self::ViewerInput { node, .. } => *node,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct SavedPayloadSubscription {
    target: SavedPayloadTarget,
    payload: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct SavedPayloadSubscriptions {
    version: u32,
    subscriptions: Vec<SavedPayloadSubscription>,
}

struct DiscoveredPayloadSubscription {
    target: SavedPayloadTarget,
    label: String,
    current_payload: Option<String>,
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
    registry: GraphRegistry,
    subscribable_kinds: HashSet<PortKind>,
    payload_ids: HashMap<PortKind, String>,
    registered_payload_ids: HashSet<String>,
    legacy_viewer_names: HashSet<String>,
}

impl SelectionContracts {
    fn from_inventory() -> Self {
        let node_registrations = graph_node_registrations();
        let legacy_viewer_names = node_registrations
            .iter()
            .filter(|registration| registration.stable_id() == LEGACY_VIEWER_NODE_ID)
            .map(|registration| registration.name().to_owned())
            .collect();
        let registry =
            GraphRegistry::with_capability_overrides_and_infrastructure(Vec::new(), Vec::new());
        let registrations = payload_registrations();
        let subscribable_kinds = registrations
            .iter()
            .map(|registration| registration.kind())
            .collect();
        let payload_ids = registrations
            .iter()
            .map(|registration| (registration.kind(), registration.stable_id().to_owned()))
            .collect();
        let registered_payload_ids = registrations
            .iter()
            .map(|registration| registration.stable_id().to_owned())
            .collect();
        Self {
            registry,
            subscribable_kinds,
            payload_ids,
            registered_payload_ids,
            legacy_viewer_names,
        }
    }
}

fn saved_map(graph: &GraphState) -> HashMap<SavedEndpoint, bool> {
    graph
        .extension::<SavedSelections>(EXTENSION)
        .ok()
        .flatten()
        .filter(|saved| saved.version == VERSION)
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
        let presentation = contracts.registry.presentation(node.def_name());
        let semantics = contracts.registry.semantics(node.def_name());
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
            let (default_selected, indicator_outputs) =
                if let (Some(presentation), Some(semantics)) = (presentation, semantics) {
                    let output_reference = node
                        .socket_reference(SocketDirection::Output, output_index)
                        .expect("enumerated output has a semantic reference");
                    let Some(ViewerOutputControl::Selectable {
                        default_selected,
                        indicator_outputs,
                    }) = presentation.viewer_output_control(output_reference, &node.state)
                    else {
                        continue;
                    };
                    let viewable = presentation
                        .viewer_channel_origin(output_reference, &node.state)
                        .is_some()
                        || semantics
                            .offered_kinds(output_reference, &node.state)
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

/// Builds the application-owned retention and presentation plan used at every
/// graph execution boundary.
///
/// Selected outputs are presented. Outputs owned by a node participating in
/// the processing graph remain retained so presentation-only edits can use
/// cached data without rerunning the graph.
pub(crate) fn output_subscription_plan(graph: &GraphState) -> OutputSubscriptionPlan {
    let connected_nodes = graph
        .connections
        .iter()
        .flat_map(|connection| [connection.from.node, connection.to.node])
        .collect::<HashSet<_>>();
    let mut subscriptions = OutputSubscriptionPlan::new();
    for selection in viewer_output_selections(graph) {
        if selection.selected || connected_nodes.contains(&selection.node) {
            subscriptions.retain(selection.node, selection.output);
        }
        if selection.selected {
            subscriptions.subscribe(selection.node, selection.output);
        }
    }
    subscriptions
}

pub(crate) fn synchronize_viewer_selections(
    graph: &mut GraphState,
) -> Result<Vec<ViewerSelectionWarning>, serde_json::Error> {
    let saved = match graph.extension::<SavedSelections>(EXTENSION) {
        Ok(saved) => saved,
        Err(error) => {
            return Ok(vec![ViewerSelectionWarning {
                node: None,
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
            node: None,
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
            node: None,
            message: "Migrated legacy socket viewer selections to the LogicConduit viewer-selection manifest"
                .to_owned(),
        })
        .into_iter()
        .collect())
}

pub(crate) fn synchronize_viewer_compatibility(
    graph: &mut GraphState,
) -> Result<Vec<ViewerSelectionWarning>, serde_json::Error> {
    let mut warnings = migrate_legacy_viewer_nodes(graph)?;
    warnings.extend(synchronize_viewer_selections(graph)?);
    warnings.extend(synchronize_payload_subscriptions(graph)?);
    Ok(warnings)
}

fn migrate_legacy_viewer_nodes(
    graph: &mut GraphState,
) -> Result<Vec<ViewerSelectionWarning>, serde_json::Error> {
    let contracts = SelectionContracts::from_inventory();
    let viewer_nodes = graph
        .nodes
        .iter()
        .filter(|(_, node)| contracts.legacy_viewer_names.contains(node.def_name()))
        .map(|(&node, _)| node)
        .collect::<HashSet<_>>();
    if viewer_nodes.is_empty() {
        return Ok(Vec::new());
    }
    let viewer_schema = graph.extension::<SavedSelections>(EXTENSION);
    let payload_schema = graph.extension::<SavedPayloadSubscriptions>(PAYLOAD_EXTENSION);
    if !matches!(
        viewer_schema,
        Ok(None)
            | Ok(Some(SavedSelections {
                version: VERSION,
                ..
            }))
    ) || !matches!(
        payload_schema,
        Ok(None)
            | Ok(Some(SavedPayloadSubscriptions {
                version: PAYLOAD_VERSION,
                ..
            }))
    ) {
        return Ok(vec![ViewerSelectionWarning {
            node: None,
            message: "Legacy Viewer nodes were preserved because their saved selection or payload manifest has an invalid or unsupported version"
                .to_owned(),
        }]);
    }

    let source_by_input = graph
        .connections
        .iter()
        .filter(|connection| viewer_nodes.contains(&connection.to.node))
        .filter_map(|connection| {
            resolve_source(graph, connection.from, 0)
                .map(|source| ((connection.to.node, connection.to.index), source))
        })
        .collect::<HashMap<_, _>>();
    let selected_sources = source_by_input.values().copied().collect::<HashSet<_>>();
    let mut selections = viewer_output_selections(graph);
    for selection in &mut selections {
        if selected_sources.contains(&SocketId {
            node: selection.node,
            index: selection.output,
            direction: SocketDirection::Output,
        }) {
            selection.selected = true;
        }
    }
    store(graph, &selections)?;
    if let Ok(Some(mut payloads)) = graph.extension::<SavedPayloadSubscriptions>(PAYLOAD_EXTENSION)
        && payloads.version == PAYLOAD_VERSION
    {
        for subscription in &mut payloads.subscriptions {
            let SavedPayloadTarget::ViewerInput { node, input } = subscription.target else {
                continue;
            };
            let Some(source) = source_by_input.get(&(node, input)) else {
                continue;
            };
            subscription.target = SavedPayloadTarget::ShowInView {
                node: source.node,
                output: source.index,
            };
        }
        graph.set_extension(PAYLOAD_EXTENSION, payloads)?;
    }
    let count = viewer_nodes.len();
    for node in viewer_nodes {
        graph.remove_node(node);
    }
    Ok(vec![ViewerSelectionWarning {
        node: None,
        message: format!("Migrated {count} legacy Viewer node(s) to UI-owned output subscriptions"),
    }])
}

fn synchronize_payload_subscriptions(
    graph: &mut GraphState,
) -> Result<Vec<ViewerSelectionWarning>, serde_json::Error> {
    let contracts = SelectionContracts::from_inventory();
    let mut warnings = Vec::new();
    let saved = match graph.extension::<SavedPayloadSubscriptions>(PAYLOAD_EXTENSION) {
        Ok(saved) => saved,
        Err(error) => {
            return Ok(vec![ViewerSelectionWarning {
                node: None,
                message: format!(
                    "Could not read the saved payload-subscription manifest; it was preserved unchanged: {error}"
                ),
            }]);
        }
    };
    let legacy = saved.is_none();
    if let Some(saved) = &saved
        && saved.version != PAYLOAD_VERSION
    {
        return Ok(vec![ViewerSelectionWarning {
            node: None,
            message: format!(
                "Payload-subscription manifest version {} is not supported by version {}; it was preserved unchanged",
                saved.version, PAYLOAD_VERSION
            ),
        }]);
    }
    let previous: HashMap<_, _> = saved
        .into_iter()
        .flat_map(|saved| saved.subscriptions)
        .map(|subscription| (subscription.target, subscription.payload))
        .collect();
    let discovered = discover_payload_subscriptions(graph, &contracts);
    let mut subscriptions = Vec::with_capacity(discovered.len());
    let mut migrated = 0;

    for discovered in discovered {
        let previous_payload = previous.get(&discovered.target);
        let payload = match (&discovered.current_payload, previous_payload) {
            (Some(current), Some(previous)) if current != previous => {
                warnings.push(ViewerSelectionWarning {
                    node: Some(discovered.target.node()),
                    message: format!(
                        "{} was saved for payload '{}' but now provides '{}'; the current registered presentation is used",
                        discovered.label, previous, current
                    ),
                });
                current.clone()
            }
            (Some(current), _) => {
                if legacy || previous_payload.is_none() {
                    migrated += 1;
                }
                current.clone()
            }
            (None, Some(previous)) => {
                let message = if !contracts.registered_payload_ids.contains(previous) {
                    format!(
                        "{} needs payload '{}', but that payload is not registered; install or enable its plugin",
                        discovered.label, previous
                    )
                } else {
                    format!(
                        "{} could not resolve its saved payload '{}' from the current source output",
                        discovered.label, previous
                    )
                };
                warnings.push(ViewerSelectionWarning {
                    node: Some(discovered.target.node()),
                    message,
                });
                previous.clone()
            }
            (None, None) => {
                warnings.push(ViewerSelectionWarning {
                    node: Some(discovered.target.node()),
                    message: format!(
                        "{} has no registered collection/presentation contract and could not be migrated",
                        discovered.label
                    ),
                });
                continue;
            }
        };
        subscriptions.push(SavedPayloadSubscription {
            target: discovered.target,
            payload,
        });
    }

    if migrated > 0 {
        warnings.push(ViewerSelectionWarning {
            node: None,
            message: format!(
                "Migrated {migrated} legacy Viewer lane selection(s) to stable payload identities; their visual presentation was preserved"
            ),
        });
    }
    if subscriptions.is_empty() {
        graph.remove_extension(PAYLOAD_EXTENSION);
    } else {
        graph.set_extension(
            PAYLOAD_EXTENSION,
            SavedPayloadSubscriptions {
                version: PAYLOAD_VERSION,
                subscriptions,
            },
        )?;
    }
    Ok(warnings)
}

fn discover_payload_subscriptions(
    graph: &GraphState,
    contracts: &SelectionContracts,
) -> Vec<DiscoveredPayloadSubscription> {
    let mut discovered = viewer_output_selections(graph)
        .into_iter()
        .filter(|selection| selection.selected)
        .filter_map(|selection| {
            let node = graph.nodes.get(&selection.node)?;
            let output = node.outputs.get(selection.output)?;
            Some(discover_payload_subscription(
                graph,
                contracts,
                SavedPayloadTarget::ShowInView {
                    node: selection.node,
                    output: selection.output,
                },
                SocketId {
                    node: selection.node,
                    index: selection.output,
                    direction: SocketDirection::Output,
                },
                format!("View selection '{}.{}'", node.title, output.name),
            ))
        })
        .collect::<Vec<_>>();

    for connection in &graph.connections {
        let Some(target) = graph.nodes.get(&connection.to.node) else {
            continue;
        };
        if !contracts
            .registry
            .semantics(target.def_name())
            .is_some_and(|semantics| semantics.is_data_subscription())
        {
            continue;
        }
        let input_name = target
            .inputs
            .get(connection.to.index)
            .map(|input| input.name.as_str())
            .unwrap_or("?");
        let source = resolve_source(graph, connection.from, 0).unwrap_or(connection.from);
        discovered.push(discover_payload_subscription(
            graph,
            contracts,
            SavedPayloadTarget::ViewerInput {
                node: connection.to.node,
                input: connection.to.index,
            },
            source,
            format!("Viewer input '{}.{}'", target.title, input_name),
        ));
    }
    discovered.sort_by_key(|subscription| match subscription.target {
        SavedPayloadTarget::ShowInView { node, output } => (node.0, 0, output),
        SavedPayloadTarget::ViewerInput { node, input } => (node.0, 1, input),
    });
    discovered
}

fn discover_payload_subscription(
    graph: &GraphState,
    contracts: &SelectionContracts,
    target: SavedPayloadTarget,
    source: SocketId,
    label: String,
) -> DiscoveredPayloadSubscription {
    let current_payload = graph
        .nodes
        .get(&source.node)
        .filter(|node| node.kind == NodeKind::Regular)
        .and_then(|node| {
            let semantics = contracts.registry.semantics(node.def_name())?;
            let output = node.socket_reference(SocketDirection::Output, source.index)?;
            semantics
                .offered_kinds(output, &node.state)
                .into_iter()
                .find_map(|kind| contracts.payload_ids.get(&kind).cloned())
        });
    DiscoveredPayloadSubscription {
        target,
        label,
        current_payload,
    }
}

fn resolve_source(graph: &GraphState, from: SocketId, depth: usize) -> Option<SocketId> {
    if depth > graph.connections.len() + graph.nodes.len() {
        return None;
    }
    let node = graph.nodes.get(&from.node)?;
    if node.kind == NodeKind::Reroute {
        let connection = graph
            .connections
            .iter()
            .find(|connection| connection.to.node == from.node)?;
        return resolve_source(graph, connection.from, depth + 1);
    }
    if node.muted {
        let pass_through_pairs = node.mute_pass_through_pairs();
        let input_index = pass_through_pairs
            .iter()
            .find(|(output_index, _)| *output_index == from.index)?
            .1;
        let connection = graph.connections.iter().find(|connection| {
            connection.to
                == SocketId {
                    node: from.node,
                    index: input_index,
                    direction: SocketDirection::Input,
                }
        })?;
        return resolve_source(graph, connection.from, depth + 1);
    }
    Some(from)
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

#[cfg(test)]
mod viewer_selection_tests {
    use node_graph::NodeGraphWidget;

    use super::*;
    use crate::test_contracts_tests::{WORD_PRODUCER_NAME, build_test_node_registry};

    #[test]
    fn unsupported_extension_versions_are_preserved_verbatim() {
        let mut widget = NodeGraphWidget::new(build_test_node_registry());
        let legacy_viewer = widget
            .add_node_at("Viewer", egui::Pos2::ZERO)
            .expect("legacy Viewer node should be registered");
        let graph = widget.graph_mut();
        let viewer_value = serde_json::json!({
            "version": VERSION + 1,
            "selections": [],
            "future_owner_data": {"keep": true}
        });
        let payload_value = serde_json::json!({
            "version": PAYLOAD_VERSION + 1,
            "subscriptions": [],
            "future_owner_data": {"keep": true}
        });
        graph.set_extension(EXTENSION, &viewer_value).unwrap();
        graph
            .set_extension(PAYLOAD_EXTENSION, &payload_value)
            .unwrap();

        let warnings = synchronize_viewer_compatibility(graph).unwrap();

        assert_eq!(
            graph.extension::<serde_json::Value>(EXTENSION).unwrap(),
            Some(viewer_value)
        );
        assert_eq!(
            graph
                .extension::<serde_json::Value>(PAYLOAD_EXTENSION)
                .unwrap(),
            Some(payload_value)
        );
        assert_eq!(
            warnings
                .iter()
                .filter(|warning| warning.message.contains("preserved unchanged"))
                .count(),
            2
        );
        assert!(graph.nodes.contains_key(&legacy_viewer));
    }

    #[test]
    fn invalid_payload_extension_is_preserved_verbatim() {
        let mut graph = GraphState::default();
        let invalid = serde_json::json!({
            "version": PAYLOAD_VERSION,
            "subscriptions": "owned by a schema this build cannot decode"
        });
        graph.set_extension(PAYLOAD_EXTENSION, &invalid).unwrap();

        let warnings = synchronize_viewer_compatibility(&mut graph).unwrap();

        assert_eq!(
            graph
                .extension::<serde_json::Value>(PAYLOAD_EXTENSION)
                .unwrap(),
            Some(invalid)
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("preserved unchanged"))
        );
    }

    #[test]
    fn legacy_socket_selection_migrates_with_visible_warning() {
        let mut widget = NodeGraphWidget::new(build_test_node_registry());
        let decoder = widget
            .add_node_at(WORD_PRODUCER_NAME, egui::Pos2::ZERO)
            .unwrap();
        widget.graph_mut().nodes.get_mut(&decoder).unwrap().outputs[0]
            .extensions
            .insert("show_in_view".to_owned(), serde_json::json!(true));

        let warnings = synchronize_viewer_compatibility(widget.graph_mut()).unwrap();
        let payloads: SavedPayloadSubscriptions = widget
            .graph()
            .extension(PAYLOAD_EXTENSION)
            .unwrap()
            .unwrap();

        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("legacy socket viewer selections"))
        );
        assert_eq!(
            payloads.subscriptions[0].payload,
            "org.logicconduit.word/v1"
        );
        assert!(
            !widget.graph().nodes[&decoder].outputs[0]
                .extensions
                .contains_key("show_in_view")
        );
    }

    #[test]
    fn legacy_viewer_node_becomes_ui_owned_output_subscription() {
        let mut widget = NodeGraphWidget::new(build_test_node_registry());
        let decoder = widget
            .add_node_at(WORD_PRODUCER_NAME, egui::Pos2::ZERO)
            .unwrap();
        let viewer = widget
            .add_node_at("Viewer", egui::Pos2::new(200.0, 0.0))
            .unwrap();
        widget.graph_mut().add_connection(
            SocketId {
                node: decoder,
                index: 0,
                direction: SocketDirection::Output,
            },
            SocketId {
                node: viewer,
                index: 0,
                direction: SocketDirection::Input,
            },
        );
        widget
            .graph_mut()
            .set_extension(
                PAYLOAD_EXTENSION,
                SavedPayloadSubscriptions {
                    version: PAYLOAD_VERSION,
                    subscriptions: vec![SavedPayloadSubscription {
                        target: SavedPayloadTarget::ViewerInput {
                            node: viewer,
                            input: 0,
                        },
                        payload: "org.logicconduit.word/v1".to_owned(),
                    }],
                },
            )
            .unwrap();

        let warnings = synchronize_viewer_compatibility(widget.graph_mut()).unwrap();

        assert!(!widget.graph().nodes.contains_key(&viewer));
        assert!(
            viewer_output_selections(widget.graph())
                .iter()
                .any(|selection| selection.node == decoder
                    && selection.output == 0
                    && selection.selected)
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("legacy Viewer node"))
        );
        let payloads: SavedPayloadSubscriptions = widget
            .graph()
            .extension(PAYLOAD_EXTENSION)
            .unwrap()
            .unwrap();
        assert!(matches!(
            payloads.subscriptions[0].target,
            SavedPayloadTarget::ShowInView { node, output: 0 } if node == decoder
        ));
    }

    #[test]
    fn execution_plan_retains_connected_outputs_and_presents_only_selections() {
        let mut widget = NodeGraphWidget::new(build_test_node_registry());
        let producer = widget
            .add_node_at(WORD_PRODUCER_NAME, egui::Pos2::ZERO)
            .unwrap();
        let viewer = widget
            .add_node_at("Viewer", egui::Pos2::new(200.0, 0.0))
            .unwrap();
        widget.graph_mut().add_connection(
            SocketId {
                node: producer,
                index: 0,
                direction: SocketDirection::Output,
            },
            SocketId {
                node: viewer,
                index: 0,
                direction: SocketDirection::Input,
            },
        );

        let retained = output_subscription_plan(widget.graph());
        assert!(retained.is_retained(producer, 0));
        assert!(!retained.contains(producer, 0));

        let output_id = viewer_output_selections(widget.graph())[0]
            .output_id
            .clone();
        set_viewer_output_selected(widget.graph_mut(), producer, &output_id, true).unwrap();
        let selected = output_subscription_plan(widget.graph());
        assert!(selected.is_retained(producer, 0));
        assert!(selected.contains(producer, 0));
    }
}
