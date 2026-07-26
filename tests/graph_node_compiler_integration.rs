use logic_analyzer_graph_api::node::{RuntimeBuilder, graph_node_registrations};
use logic_analyzer_graph_api::node_support::{CapturePresentation, ViewerOutputControl};
use logic_analyzer_graph_compiler::{CompiledGraph, GraphCompiler};
use logic_analyzer_graph_nodes::test_support as nodes;
use node_graph::{GraphState, NodeGraphWidget, NodeId};

fn selected_output_nodes(graph: &GraphState) -> Vec<NodeId> {
    let builders: std::collections::HashMap<String, Box<dyn RuntimeBuilder>> =
        graph_node_registrations()
            .into_iter()
            .filter_map(|registration| {
                registration
                    .builder()
                    .map(|builder| (registration.name().to_owned(), builder))
            })
            .collect();
    graph
        .nodes
        .iter()
        .flat_map(|(&node_id, node)| {
            let builder = builders.get(node.def_name());
            node.outputs.iter().filter_map(move |output| {
                let builder = builder?;
                let ViewerOutputControl::Selectable {
                    default_selected, ..
                } = builder.viewer_output_control(output, &node.state)?
                else {
                    return None;
                };
                let selected = output
                    .extensions
                    .get("show_in_view")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(default_selected);
                selected.then_some(node_id)
            })
        })
        .collect()
}

#[test]
fn binary_decoder_demo_fixture_lowers_with_built_in_nodes() {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::build_binary_decoder_demo(&mut widget);
    let source_name = nodes::node_name("org.logicconduit.graph-node.sigrok-file-source/v1");
    let compiler = GraphCompiler::new();
    let selected_nodes = selected_output_nodes(widget.graph());
    let raw_channels = selected_nodes
        .iter()
        .filter(|node| widget.graph().nodes[node].def_name() == source_name)
        .count();
    let derived_lanes = selected_nodes
        .iter()
        .filter(|node| widget.graph().nodes[node].def_name() != source_name)
        .count();
    assert_eq!(raw_channels, 11);
    assert_eq!(derived_lanes, 0);

    let preview = compiler
        .discover_capture_presentation(widget.graph())
        .unwrap()
        .expect("demo source should provide a pre-run capture preview");
    let CapturePresentation::InMemory {
        signals: preview, ..
    } = preview.presentation
    else {
        panic!("demo source should provide an in-memory presentation");
    };
    assert_eq!(preview.len(), 10);
    assert_eq!(preview.first().unwrap().name, "Ch 0");
    assert_eq!(preview.last().unwrap().name, "Ch 10");
    assert_eq!(
        preview.last().unwrap().transitions.last().unwrap().0,
        59_999_000.0
    );

    let compiled = compiler
        .lower(widget.graph())
        .expect("demo should lower cleanly");
    assert_eq!(widget.graph().nodes.len(), 9);
    assert_eq!(compiled.nodes.len(), 8);
}

#[test]
fn built_in_graph_json_round_trip_compiles_identically() {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::populate_startup(&mut widget);
    let compiler = GraphCompiler::new();
    let original = compiler.lower(widget.graph()).expect("original lowers");

    let json = serde_json::to_string(widget.graph()).expect("graph serializes");
    let restored_state: GraphState = serde_json::from_str(&json).expect("graph deserializes");
    let mut restored = NodeGraphWidget::new(nodes::build_registry());
    restored.set_graph(restored_state);

    let reloaded = compiler.lower(restored.graph()).expect("restored lowers");

    assert_eq!(original.nodes.len(), reloaded.nodes.len());
    for (before, after) in original.nodes.iter().zip(&reloaded.nodes) {
        assert_eq!(before.id, after.id);
        assert_eq!(before.builder, after.builder);
        assert_eq!(before.state, after.state);
    }
    assert_eq!(compiled_edges(&original), compiled_edges(&reloaded));
}

fn compiled_edges(compiled: &CompiledGraph) -> Vec<String> {
    let mut edges = compiled
        .edges
        .iter()
        .map(|edge| {
            format!(
                "n{}:{} -> n{}:{} ({})",
                edge.from.0.0, edge.from.1, edge.to.0.0, edge.to.1, edge.buffer
            )
        })
        .collect::<Vec<_>>();
    edges.sort();
    edges
}
