//! Portable scale regression fixtures, with measurements printed for release runs.

use std::collections::BTreeMap;
use std::hint::black_box;

use egui::{Pos2, Rect, Vec2};
use web_time::Instant;

use super::interaction_state::InteractionState;
use super::routing::{PathSegment, RouteConfig};
use super::routing_cache::RoutingCache;
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
    serde_json::json!({"p50_ms": at(50), "p95_ms": at(95), "p99_ms": at(99), "max_ms": samples.last(), "samples_ms": samples})
}

fn measure(mut progress: impl FnMut(serde_json::Value)) -> serde_json::Value {
    let mut reports = Vec::new();
    for (node_count, connection_count) in [(100, 500), (500, 2000)] {
        progress(serde_json::json!({"nodes": node_count, "phase": "construct"}));
        let mut widget = fixture(node_count, connection_count);
        let context = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0));
        let mut routing = Vec::new();
        let mut cached_routing = Vec::new();
        let mut hover = Vec::new();
        let mut frame = Vec::new();
        let mut layout_times = Vec::new();
        let mut ui_times = Vec::new();
        let mut tessellation_times = Vec::new();
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
            let start = Instant::now();
            let mut layout = widget.build_layout(Pos2::ZERO);
            let layout_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            layout.rebuild_routes(&widget.graph.connections, widget.view.zoom);
            let route_ms = start.elapsed().as_secs_f64() * 1000.0;
            progress(
                serde_json::json!({"nodes": node_count, "sample": sample, "phase": "routed", "routing_ms": route_ms}),
            );
            assert_eq!(layout.wire_paths.len(), connection_count);
            assert!(
                layout.wire_failures.is_empty(),
                "the disjoint scale fixture must complete checked cold routing within the default work budget"
            );
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
            let reused = widget.routing_cache.borrow_mut().route(
                &mut layout,
                &widget.graph.connections,
                &RouteConfig::default(),
                widget.view.zoom,
            );
            let cached_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert!(
                reused,
                "the stationary fixture must reuse identical routing inputs"
            );
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
            let ui_ms = start.elapsed().as_secs_f64() * 1000.0;
            // This CPU-only harness deliberately does not upload textures.
            output.textures_delta.clear();
            let tessellation_start = Instant::now();
            let meshes = context.tessellate(output.shapes, output.pixels_per_point);
            assert!(!meshes.is_empty());
            black_box(meshes);
            let tessellation_ms = tessellation_start.elapsed().as_secs_f64() * 1000.0;
            let frame_ms = start.elapsed().as_secs_f64() * 1000.0;
            progress(
                serde_json::json!({"nodes": node_count, "sample": sample, "phase": "frame", "cpu_frame_ms": frame_ms}),
            );
            if sample == 0 {
                first = Some(
                    serde_json::json!({"routing_ms": route_ms, "cached_routing_ms": cached_ms, "hover_ms": hover_ms, "cpu_frame_ms": frame_ms, "layout_ms": layout_ms, "ui_ms": ui_ms, "tessellation_ms": tessellation_ms}),
                );
            } else {
                routing.push(route_ms);
                cached_routing.push(cached_ms);
                hover.push(hover_ms);
                frame.push(frame_ms);
                layout_times.push(layout_ms);
                ui_times.push(ui_ms);
                tessellation_times.push(tessellation_ms);
            }
            assert_eq!(widget.graph.nodes.len(), node_count);
            assert_eq!(widget.graph.connections.len(), connection_count);
        }
        let (failures, cubics) = outcome.unwrap();
        reports.push(serde_json::json!({
            "nodes": node_count, "connections": connection_count, "fallbacks": failures,
            "cubic_segments": cubics, "failure_reasons": failure_reasons, "first": first,
            "routing": distribution(routing), "cached_routing": distribution(cached_routing),
            "hover": distribution(hover), "cpu_frame": distribution(frame),
            "layout": distribution(layout_times), "ui": distribution(ui_times),
            "tessellation": distribution(tessellation_times),
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

fn measure_drag() -> serde_json::Value {
    let mut reports = Vec::new();
    for (node_count, connection_count) in [(100, 500), (500, 2000)] {
        let mut widget = fixture(node_count, connection_count);
        let moving = widget.graph.connections[0].from.node;
        widget.interaction_state = InteractionState::DraggingNode {
            node_id: moving,
            offset: Vec2::ZERO,
            constraint: None,
        };
        let config = RouteConfig::default();
        let zoom = widget.view.zoom;
        let mut layout = widget.build_layout(Pos2::ZERO);
        // Independent history isolates route-update timing from layout preparation.
        let mut history = RoutingCache::default();
        history.route_interactive(&mut layout, &widget.graph.connections, &config, zoom, true);
        let mut previous = layout.wire_paths.clone();
        let mut updates = Vec::new();
        let mut cold = Vec::new();
        let mut layouts = Vec::new();
        let mut outcomes = Vec::new();
        let samples = if cfg!(debug_assertions) { 2 } else { 21 };
        let mut first = None;
        for sample in 0..samples {
            widget.graph.nodes.get_mut(&moving).unwrap().pos.y =
                if sample % 2 == 0 { 20.0 } else { 0.0 };
            let start = Instant::now();
            layout = widget.build_layout(Pos2::ZERO);
            let layout_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let exact_hit = history.route_interactive(
                &mut layout,
                &widget.graph.connections,
                &config,
                zoom,
                true,
            );
            let update_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert!(!exact_hit, "every sample changes an endpoint's geometry");
            assert_eq!(layout.wire_paths.len(), connection_count);
            assert!(layout.wire_paths.values().all(|p| p.bounds().is_finite()));
            let mut retained = 0;
            for (key, path) in &layout.wire_paths {
                if previous[key].segments().as_ptr() == path.segments().as_ptr() {
                    assert_ne!(key.0.node, moving, "incident paths must be rebuilt");
                    assert_ne!(key.1.node, moving);
                    assert!(!layout.wire_failures.contains_key(key));
                    retained += 1;
                }
            }
            assert!(retained > 0, "unrelated checked pairs must remain shared");
            previous.clone_from(&layout.wire_paths);
            let mut warm_failures = BTreeMap::new();
            for reason in layout.wire_failures.values() {
                *warm_failures
                    .entry(format!("{reason:?}"))
                    .or_insert(0_usize) += 1;
            }
            let start = Instant::now();
            layout.rebuild_routes(&widget.graph.connections, zoom);
            let cold_ms = start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(layout.wire_paths.len(), connection_count);
            assert!(
                layout.wire_failures.is_empty(),
                "connected-endpoint moves must retain complete cold/release routing"
            );
            assert!(layout.wire_paths.values().all(|p| p.bounds().is_finite()));
            let mut cold_failures = BTreeMap::new();
            for reason in layout.wire_failures.values() {
                *cold_failures
                    .entry(format!("{reason:?}"))
                    .or_insert(0_usize) += 1;
            }
            outcomes.push(serde_json::json!({"sample": sample, "retained_paths": retained, "warm_failures": warm_failures, "cold_failures": cold_failures}));
            if sample == 0 {
                first = Some(
                    serde_json::json!({"update_ms": update_ms, "cold_ms": cold_ms, "layout_ms": layout_ms}),
                );
            } else {
                updates.push(update_ms);
                cold.push(cold_ms);
                layouts.push(layout_ms);
            }
        }
        let expected = layout.wire_paths.clone();
        let failures = layout.wire_failures.clone();
        let start = Instant::now();
        assert!(!history.route_interactive(
            &mut layout,
            &widget.graph.connections,
            &config,
            zoom,
            false
        ));
        let release_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(failures, layout.wire_failures);
        for (key, path) in expected {
            assert_eq!(
                format!("{:?}", path.segments()),
                format!("{:?}", layout.wire_paths[&key].segments())
            );
        }
        reports.push(
            serde_json::json!({"nodes": node_count, "connections": connection_count,
            "first": first, "update": distribution(updates), "cold": distribution(cold),
            "layout": distribution(layouts), "release_ms": release_ms, "outcomes": outcomes}),
        );
    }
    serde_json::json!({"fixture": "paired-grid-connected-drag-v1", "zoom": 0.35, "reports": reports})
}

#[test]
fn routing_drag_native() {
    println!("ROUTING_DRAG_PERFORMANCE {}", measure_drag());
}

#[wasm_bindgen_test::wasm_bindgen_test]
fn routing_drag_browser() {
    wasm_bindgen_test::console_log!("ROUTING_DRAG_PERFORMANCE {}", measure_drag());
}
