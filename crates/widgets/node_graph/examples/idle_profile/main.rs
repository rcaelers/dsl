//! Opt-in CPU workloads for external sampling; no GPU or capture I/O.

use std::hint::black_box;
use std::sync::Arc;

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Vec2};

use input_bindings::InputBindings;
use node_graph::NodeGraphWidget;
use node_graph::api::{
    AnySocket, InputDef, NodeDef, NodeId, NodeTypeRegistry, OutputDef, SocketDirection, SocketId,
};

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

fn frame(widget: &mut NodeGraphWidget, context: &egui::Context, index: usize, zoom: Option<f32>) {
    let pointer = if zoom.is_some() {
        Pos2::ZERO
    } else {
        Pos2::new(320.0, 160.0)
    };
    let mut events = vec![
        Event::ModifiersChanged(if zoom.is_some() {
            Modifiers::CTRL
        } else {
            Modifiers::NONE
        }),
        Event::PointerMoved(pointer),
    ];
    if let Some(zoom) = zoom {
        events.push(Event::Zoom(zoom));
    }
    frame_events(widget, context, index, events);
}

fn frame_events(
    widget: &mut NodeGraphWidget,
    context: &egui::Context,
    index: usize,
    events: Vec<Event>,
) {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440.0, 900.0))),
            time: Some(index as f64 / 60.0),
            events,
            ..Default::default()
        },
        |ui| {
            black_box(widget.show(ui));
        },
    );
    output.textures_delta.clear();
    let meshes = context.tessellate(output.shapes, output.pixels_per_point);
    assert!(!meshes.is_empty());
    black_box(meshes);
}

fn fixture(pairs: usize, offset: Vec2) -> NodeGraphWidget {
    let mut registry = NodeTypeRegistry::new();
    registry.register::<FixtureNode>();
    let mut widget = NodeGraphWidget::new(registry);
    // Same paired-grid-v1 geometry and ten sockets per direction as the scale tests.
    for pair in 0..pairs {
        let origin = Pos2::new((pair % 5) as f32 * 900.0, (pair / 5) as f32 * 700.0) + offset;
        let source = widget.add_node_at(FixtureNode::name(), origin).unwrap();
        let target = widget
            .add_node_at(FixtureNode::name(), origin + Vec2::new(450.0, 0.0))
            .unwrap();
        for index in 0..8 {
            widget.graph_mut().add_connection(
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
    widget
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

// Keep phase boundaries visible in optimized external stack samples. These
// wrappers are profiling-fixture code, not hooks in the production widget.
#[inline(never)]
fn prepare_release(
    widget: &mut NodeGraphWidget,
    context: &egui::Context,
    moving: NodeId,
    index: usize,
    dy: f32,
) -> Pos2 {
    // This fixed fixture's header interior is 20 px right and 4 px down from
    // its top-left at 35% zoom. A translated grid keeps it away from auto-pan.
    let position = widget.graph().nodes[&moving].pos;
    let grip = Pos2::new(position.x, position.y) * 0.35 + Vec2::new(20.0, 4.0);
    frame_events(
        widget,
        context,
        index,
        vec![
            Event::ModifiersChanged(Modifiers::NONE),
            Event::PointerMoved(grip),
        ],
    );
    frame_events(widget, context, index + 1, vec![button(grip, true)]);
    let anchor = grip + Vec2::new(0.0, 10.0);
    frame_events(
        widget,
        context,
        index + 2,
        vec![Event::PointerMoved(anchor)],
    );
    assert_eq!(widget.active_input_context(), Some("node_graph.drag_node"));
    let initial = widget.graph().nodes[&moving].pos;
    let pointer = anchor + Vec2::new(0.0, dy * 0.35);
    frame_events(
        widget,
        context,
        index + 3,
        vec![Event::PointerMoved(pointer)],
    );
    let final_position = widget.graph().nodes[&moving].pos;
    assert!(
        (final_position.y - initial.y - dy).abs() < 0.001,
        "{initial:?} -> {final_position:?}, dy {dy}"
    );
    assert!((final_position.x - initial.x).abs() < 0.001);
    pointer
}

#[inline(never)]
fn release_frame(widget: &mut NodeGraphWidget, context: &egui::Context, index: usize, pos: Pos2) {
    frame_events(widget, context, index, vec![button(pos, false)]);
    black_box(widget.zoom_percent()); // Keep this phase's stack frame across the call.
}

#[inline(never)]
fn settled_frame(widget: &mut NodeGraphWidget, context: &egui::Context, index: usize) {
    frame_events(widget, context, index, Vec::new());
    black_box(widget.zoom_percent());
}

fn release_cycles(widget: &mut NodeGraphWidget, context: &egui::Context, cycles: usize) {
    let moving = widget.graph().connections[0].from.node;
    let original = widget.graph().clone();
    for cycle in 0..cycles {
        let index = cycle * 6 + 3;
        let initial = widget.graph().nodes[&moving].pos;
        let dy = if cycle % 2 == 0 { 20.0 } else { -20.0 };
        let pointer = prepare_release(widget, context, moving, index, dy);
        let position = widget.graph().nodes[&moving].pos;
        assert!(
            (position.y - initial.y - dy).abs() < 0.001,
            "drag-start drift"
        );
        release_frame(widget, context, index + 4, pointer);
        assert_eq!(widget.active_input_context(), None);
        assert_eq!(widget.graph().nodes[&moving].pos, position);
        settled_frame(widget, context, index + 5);
        assert_eq!(widget.active_input_context(), None);
        assert_eq!(widget.graph().nodes[&moving].pos, position);
    }
    // Selection and the moved node's position can change. Everything else in
    // the portable graph document must remain identical.
    let mut after = widget.graph().clone();
    for (&id, node) in &mut after.nodes {
        node.selected = original.nodes[&id].selected;
        if id == moving {
            node.pos = original.nodes[&id].pos;
        }
    }
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    assert_eq!(widget.zoom_percent(), 35);
}

#[derive(Debug, PartialEq)]
enum Workload {
    Idle,
    Release,
}

fn arguments(mut args: impl Iterator<Item = String>) -> Result<(usize, Workload), &'static str> {
    let count = args.next().map_or(Ok(1000), |value| {
        value.parse::<usize>().map_err(|_| "invalid count")
    })?;
    let workload = match args.next().as_deref() {
        None | Some("idle") => Workload::Idle,
        Some("release") => Workload::Release,
        _ => return Err("unknown workload"),
    };
    if count == 0 || count > (usize::MAX - 8) / 6 || args.next().is_some() {
        return Err("invalid count or extra arguments");
    }
    Ok((count, workload))
}

fn drag_bindings() -> Arc<InputBindings> {
    Arc::new(InputBindings::from_json(r#"{"bindings":[
        {"context":"node_graph","action":"select_move","label":"Move","input":"pointer","button":"primary","gesture":"drag"},
        {"context":"node_graph.drag_node","action":"confirm_move","label":"Confirm","input":"pointer","button":"primary","gesture":"release","any_modifiers":true}
    ]}"#).unwrap())
}

fn main() {
    let (frames, workload) = arguments(std::env::args().skip(1))
        .expect("usage: idle_profile [positive-count] [idle|release]");
    // Release profiling uses a translated grid, not a private view setter.
    // This is a separate sampling fixture, not a timing-comparison baseline.
    let offset = if workload == Workload::Release {
        Vec2::new(150.0, 100.0) / 0.35
    } else {
        Vec2::ZERO
    };
    let mut widget = fixture(250, offset);
    widget.set_input_bindings(drag_bindings());
    let before = serde_json::to_value(widget.graph()).unwrap();
    let context = egui::Context::default();
    frame(&mut widget, &context, 0, None);
    // Public pointer/zoom input establishes zoom 0.35 at origin without adding
    // a profiling-only view setter to the widget's supported API.
    frame(&mut widget, &context, 1, Some(0.5));
    frame(&mut widget, &context, 2, Some(0.7));
    assert_eq!(widget.zoom_percent(), 35);
    println!(
        "Profiling {frames} {workload:?} iterations: 500 nodes / 2000 connections, zoom 0.35, viewport 1440x900"
    );
    match workload {
        Workload::Idle => {
            for index in 0..frames {
                frame(&mut widget, &context, index + 3, None);
            }
            assert_eq!(serde_json::to_value(widget.graph()).unwrap(), before);
        }
        Workload::Release => release_cycles(&mut widget, &context, frames),
    }
    assert_eq!(widget.zoom_percent(), 35);
    println!("Completed {frames} {workload:?} iterations; document invariants checked.");
}

#[cfg(test)]
mod profile_tests {
    use super::*;

    #[test]
    fn arguments_keep_idle_default_and_reject_invalid_counts_or_modes() {
        let parse = |args: &[&str]| arguments(args.iter().map(|arg| (*arg).to_owned()));
        assert_eq!(parse(&[]), Ok((1000, Workload::Idle)));
        assert_eq!(parse(&["2"]), Ok((2, Workload::Idle)));
        assert_eq!(parse(&["2", "release"]), Ok((2, Workload::Release)));
        for args in [
            vec!["0"],
            vec!["-1"],
            vec!["x"],
            vec!["2", "unknown"],
            vec!["2", "idle", "extra"],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn repeated_pointer_releases_preserve_the_fixture_document() {
        let mut widget = fixture(1, Vec2::new(150.0, 100.0) / 0.35);
        widget.set_input_bindings(drag_bindings());
        let context = egui::Context::default();
        frame(&mut widget, &context, 0, None);
        frame(&mut widget, &context, 1, Some(0.5));
        frame(&mut widget, &context, 2, Some(0.7));
        release_cycles(&mut widget, &context, 2);
    }
}
