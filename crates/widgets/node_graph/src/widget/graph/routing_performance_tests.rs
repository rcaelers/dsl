//! Portable scale regression fixtures, with measurements printed for release runs.

use std::collections::BTreeMap;
use std::hint::black_box;

use egui::{Pos2, Rect, Vec2};
use web_time::Instant;

use super::routing::PathSegment;
use super::widget::NodeGraphWidget;
use crate::api::{AnySocket, InputDef, NodeDef, OutputDef};
use crate::model::{SocketDirection, SocketId};
use crate::runtime::NodeTypeRegistry;

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

struct FixtureNode;

impl NodeDef for FixtureNode {
    type State = ();

    fn name() -> &'static str {
        "Routing fixture"
    }
    fn category() -> &'static str {
        "Test"
    }
    fn state() -> Self::State {}
    fn inputs() -> Vec<InputDef<Self::State>> {
        (0..10)
            .map(|i| InputDef::new::<AnySocket>(format!("{i}")))
            .collect()
    }
    fn outputs() -> Vec<OutputDef<Self::State>> {
        (0..10)
            .map(|i| OutputDef::new::<AnySocket>(format!("{i}")))
            .collect()
    }
}

fn fixture(nodes: usize, connections: usize) -> NodeGraphWidget {
    let mut registry = NodeTypeRegistry::new();
    registry.register::<FixtureNode>();
    let mut widget = NodeGraphWidget::new(registry);
    widget.view.zoom = 0.35;
    for pair in 0..nodes / 2 {
        let origin = Pos2::new((pair % 5) as f32 * 900.0, (pair / 5) as f32 * 700.0);
        let source = widget.add_node_at(FixtureNode::name(), origin).unwrap();
        let target = widget
            .add_node_at(FixtureNode::name(), origin + Vec2::new(450.0, 0.0))
            .unwrap();
        for index in 0..connections / (nodes / 2) {
            widget.graph.add_connection(
                SocketId {
                    node: source,
                    index,
                    direction: SocketDirection::Output,
                },
                SocketId {
                    node: target,
                    index,
                    direction: SocketDirection::Input,
                },
            );
        }
    }
    assert_eq!(widget.graph.nodes.len(), nodes);
    assert_eq!(widget.graph.connections.len(), connections);
    let layout = widget.build_layout(Pos2::ZERO);
    let bodies: Vec<_> = layout.node_rects.values().collect();
    for (i, body) in bodies.iter().enumerate() {
        for other in &bodies[i + 1..] {
            assert!(
                !body.expand(60.0).intersects(other.expand(60.0)),
                "fixture bodies and escape room must not overlap"
            );
        }
    }
    widget
}

fn distribution(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    let at = |percent: usize| samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)];
    serde_json::json!({"p50_ms": at(50), "p95_ms": at(95), "max_ms": samples.last(), "samples_ms": samples})
}

fn measure(mut progress: impl FnMut(serde_json::Value)) -> serde_json::Value {
    let mut reports = Vec::new();
    for (node_count, connection_count) in [(100, 500), (500, 2000)] {
        progress(serde_json::json!({"nodes": node_count, "phase": "construct"}));
        let mut widget = fixture(node_count, connection_count);
        let context = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0));
        let mut routing = Vec::new();
        let mut hover = Vec::new();
        let mut frame = Vec::new();
        let mut outcome = None;
        let mut failure_reasons = BTreeMap::new();
        // The first sample is reported separately: later samples warm allocator,
        // fonts and egui memory, but every routing call builds a fresh snapshot.
        let mut first = None;
        // Debug tests exercise both cold and repeated correctness without a
        // timing assertion. Release measurement runs retain twenty samples.
        let samples = if cfg!(debug_assertions) { 2 } else { 21 };
        for sample in 0..samples {
            progress(serde_json::json!({"nodes": node_count, "sample": sample, "phase": "layout"}));
            let mut layout = widget.build_layout(Pos2::ZERO);
            let start = Instant::now();
            layout.rebuild_routes(&widget.graph.connections, widget.view.zoom);
            let route_ms = start.elapsed().as_secs_f64() * 1000.0;
            progress(
                serde_json::json!({"nodes": node_count, "sample": sample, "phase": "routed", "routing_ms": route_ms}),
            );
            assert_eq!(layout.wire_paths.len(), connection_count);
            failure_reasons.clear();
            for reason in layout.wire_failures.values() {
                *failure_reasons
                    .entry(format!("{reason:?}"))
                    .or_insert(0_usize) += 1;
            }
            let counts = (
                layout.wire_failures.len(),
                layout
                    .wire_paths
                    .values()
                    .map(|path| {
                        path.segments()
                            .iter()
                            .filter(|s| matches!(s, PathSegment::Cubic(_)))
                            .count()
                    })
                    .sum::<usize>(),
            );
            assert!(layout.wire_paths.values().all(|p| p.bounds().is_finite()));
            if let Some(expected) = outcome {
                assert_eq!(counts, expected);
            }
            outcome = Some(counts);
            let start = Instant::now();
            for i in 0..32 {
                black_box(
                    widget.wire_near_point(Pos2::new(220.0 + i as f32 * 23.0, 100.0), &layout),
                );
            }
            let hover_ms = start.elapsed().as_secs_f64() * 1000.0 / 32.0;
            let start = Instant::now();
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(sample as f64 / 60.0),
                    events: vec![egui::Event::PointerMoved(Pos2::new(320.0, 160.0))],
                    ..Default::default()
                },
                |ui| {
                    black_box(widget.show(ui));
                },
            );
            // This CPU-only harness deliberately does not upload textures.
            output.textures_delta.clear();
            let meshes = context.tessellate(output.shapes, output.pixels_per_point);
            assert!(!meshes.is_empty());
            black_box(meshes);
            let frame_ms = start.elapsed().as_secs_f64() * 1000.0;
            progress(
                serde_json::json!({"nodes": node_count, "sample": sample, "phase": "frame", "cpu_frame_ms": frame_ms}),
            );
            if sample == 0 {
                first = Some(
                    serde_json::json!({"routing_ms": route_ms, "hover_ms": hover_ms, "cpu_frame_ms": frame_ms}),
                );
            } else {
                routing.push(route_ms);
                hover.push(hover_ms);
                frame.push(frame_ms);
            }
            assert_eq!(widget.graph.nodes.len(), node_count);
            assert_eq!(widget.graph.connections.len(), connection_count);
        }
        let (failures, cubics) = outcome.unwrap();
        reports.push(serde_json::json!({
            "nodes": node_count, "connections": connection_count, "fallbacks": failures,
            "cubic_segments": cubics, "failure_reasons": failure_reasons, "first": first,
            "routing": distribution(routing), "hover": distribution(hover), "cpu_frame": distribution(frame),
        }));
    }
    serde_json::json!({"fixture": "paired-grid-v1", "zoom": 0.35, "viewport": [1440, 900], "reports": reports})
}

#[test]
fn routing_scale_native() {
    println!(
        "ROUTING_PERFORMANCE {}",
        measure(|value| println!("ROUTING_PROGRESS {value}"))
    );
}

#[wasm_bindgen_test::wasm_bindgen_test]
fn routing_scale_browser() {
    wasm_bindgen_test::console_log!(
        "ROUTING_PERFORMANCE {}",
        measure(|value| wasm_bindgen_test::console_log!("ROUTING_PROGRESS {value}"))
    );
}
