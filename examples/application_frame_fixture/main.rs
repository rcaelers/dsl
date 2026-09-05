//! Explicit application-UI scale fixture generator, not a processing benchmark.
//! Reuses two reviewed nodes from a bundled document; does not define new nodes
//! or copy opaque whole-document extensions into the generated graph.

use std::io::Write;
use std::path::PathBuf;

use clap::Parser;

use node_graph::NodeGraphWidget;
use node_graph_document::{GraphPosition, GraphState, NodeId, SocketDirection, SocketId};

const TEMPLATE: &str = include_str!("../../graphs/wasm_decoder_demo.json");

#[derive(Parser)]
struct Args {
    /// New graph JSON file; existing files are never overwritten
    output: PathBuf,
    /// Number of nodes: 100 (500 wires) or 500 (2000 wires)
    #[arg(long, default_value = "100", value_parser = ["100", "500"])]
    nodes: String,
}

fn registry() -> node_graph::api::NodeTypeRegistry {
    logic_analyzer_graph_nodes::link();
    logic_analyzer_ui::build_node_registry()
}

fn fixture(nodes: usize) -> GraphState {
    assert!(matches!(nodes, 100 | 500));
    let mut template = NodeGraphWidget::new(registry());
    template.set_graph(serde_json::from_str(TEMPLATE).expect("bundled template is valid"));
    // These document identities select the bundled logic transform and parallel
    // decoder. No capture source is duplicated. This is a versioned example workload, not generic
    // behavior selected by node names or port labels in the application/widget.
    let source = &template.graph().nodes[&NodeId(7)];
    let target = &template.graph().nodes[&NodeId(8)];
    assert_eq!(source.outputs.len(), 1);
    assert!(target.inputs.len() >= 10);
    let wires = if nodes == 100 { 10 } else { 8 };
    let mut graph = GraphState::default();
    for pair in 0..nodes / 2 {
        let x = (pair % 5) as f32 * 900.0;
        let y = (pair / 5) as f32 * 700.0;
        let mut source = source.clone();
        let mut target = target.clone();
        source.id = graph.next_id();
        target.id = graph.next_id();
        source.pos = GraphPosition { x, y };
        target.pos = GraphPosition { x: x + 450.0, y };
        source.selected = false;
        target.selected = false;
        let from = source.id;
        let to = target.id;
        graph.add_node(source);
        graph.add_node(target);
        for index in 0..wires {
            graph.add_connection(
                SocketId {
                    node: from,
                    index: 0,
                    direction: SocketDirection::Output,
                },
                SocketId {
                    node: to,
                    index,
                    direction: SocketDirection::Input,
                },
            );
        }
    }
    // Explicit current-version UI document configuration, not copied opaque
    // subscriptions from the template. Deselect every output so this fixture
    // has neither legacy selection migrations nor derived-viewer work.
    let mut selections = graph
        .nodes
        .values()
        .flat_map(|node| {
            node.outputs.iter().map(|output| {
                serde_json::json!({
                    "endpoint": {"node": node.id, "output": output.schema_id}, "selected": false
                })
            })
        })
        .collect::<Vec<_>>();
    selections.sort_by_key(|selection| selection["endpoint"]["node"].as_u64().unwrap());
    graph
        .set_extension(
            "logic_analyzer_graph.viewer_selections",
            serde_json::json!({
                "version": 1, "selections": selections
            }),
        )
        .unwrap();
    let expected_connections = serde_json::to_value(&graph.connections).unwrap();
    let mut widget = NodeGraphWidget::new(registry());
    widget.set_graph(graph);
    assert_eq!(
        widget.graph().nodes.len(),
        nodes,
        "load removed fixture nodes"
    );
    assert_eq!(
        serde_json::to_value(&widget.graph().connections).unwrap(),
        expected_connections,
        "load changed fixture wires"
    );
    assert!(
        widget
            .graph()
            .nodes
            .values()
            .all(|node| node.badge.is_none()),
        "fixture has node validation badges"
    );
    widget.graph().clone()
}

fn document_bytes(graph: &GraphState) -> Vec<u8> {
    // Value uses sorted object keys, unlike GraphState's runtime HashMap.
    let mut bytes = serde_json::to_vec_pretty(&serde_json::to_value(graph).unwrap()).unwrap();
    bytes.push(b'\n');
    bytes
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let nodes = args.nodes.parse::<usize>()?;
    let graph = fixture(nodes);
    let bytes = document_bytes(&graph);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args.output)?
        .write_all(&bytes)?;
    println!(
        "APPLICATION_FRAME_FIXTURE {}",
        serde_json::json!({
            "fixture": "paired-builtins-fanout-v1", "graph": args.output,
            "graph_blake3": blake3::hash(&bytes).to_hex().to_string(),
            "template_blake3": blake3::hash(TEMPLATE.as_bytes()).to_hex().to_string(),
            "nodes": nodes, "connections": graph.connections.len(),
            "pairs_per_row": 5, "pair_stride": [900, 700], "target_offset": [450, 0],
            "scope": "stationary application UI document; existing transform/decoder definitions; shared-output fan-out; explicit deselected viewer outputs; no capture source, routing fallback or execution assertion"
        })
    );
    Ok(())
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    #[test]
    fn both_scales_are_deterministic_and_reload_without_topology_or_state_repair() {
        for (nodes, wires_per_pair) in [(100, 10), (500, 8)] {
            let graph = fixture(nodes);
            assert_eq!(graph.connections.len(), nodes / 2 * wires_per_pair);
            for (index, wire) in graph.connections.iter().enumerate() {
                let pair = index / wires_per_pair;
                assert_eq!(wire.from.node, NodeId((pair * 2) as u32));
                assert_eq!(wire.to.node, NodeId((pair * 2 + 1) as u32));
                assert_eq!(wire.from.index, 0);
                assert_eq!(wire.to.index, index % wires_per_pair);
                assert_eq!(wire.from.direction, SocketDirection::Output);
                assert_eq!(wire.to.direction, SocketDirection::Input);
            }
            let bytes = document_bytes(&graph);
            assert_eq!(bytes, document_bytes(&fixture(nodes)));
            let mut restored = NodeGraphWidget::new(registry());
            restored.set_graph(serde_json::from_slice(&bytes).unwrap());
            assert_eq!(bytes, document_bytes(restored.graph()));
            assert_eq!(restored.graph_mut().next_id(), NodeId(nodes as u32));
            for pair in 0..nodes / 2 {
                let from = &graph.nodes[&NodeId((pair * 2) as u32)];
                let to = &graph.nodes[&NodeId((pair * 2 + 1) as u32)];
                assert_eq!(from.pos.x, (pair % 5) as f32 * 900.0);
                assert_eq!(from.pos.y, (pair / 5) as f32 * 700.0);
                assert_eq!(to.pos.x, from.pos.x + 450.0);
                assert_eq!(to.pos.y, from.pos.y);
            }
        }
    }

    #[test]
    fn cli_requires_an_output_and_rejects_unsupported_scales() {
        assert!(Args::try_parse_from(["fixture"]).is_err());
        for nodes in ["0", "101", "1000", "-1"] {
            assert!(Args::try_parse_from(["fixture", "output.json", "--nodes", nodes]).is_err());
        }
    }
}
