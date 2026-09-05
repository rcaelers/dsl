//! CPU-only idle workload for an external sampling profiler; no GPU or capture I/O.

use std::hint::black_box;

use egui::{Event, Modifiers, Pos2, Rect, Vec2};

use node_graph::NodeGraphWidget;
use node_graph::api::{
    AnySocket, InputDef, NodeDef, NodeTypeRegistry, OutputDef, SocketDirection, SocketId,
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

fn main() {
    let mut args = std::env::args().skip(1);
    let frames: usize = args.next().map_or(1000, |value| {
        value.parse().expect("frame count must be an integer")
    });
    assert!(
        frames > 0 && args.next().is_none(),
        "usage: idle_profile [positive-frame-count]"
    );
    let mut registry = NodeTypeRegistry::new();
    registry.register::<FixtureNode>();
    let mut widget = NodeGraphWidget::new(registry);
    // Same paired-grid-v1 geometry and ten sockets per direction as the scale tests.
    for pair in 0..250 {
        let origin = Pos2::new((pair % 5) as f32 * 900.0, (pair / 5) as f32 * 700.0);
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
    let before = serde_json::to_value(widget.graph()).unwrap();
    let context = egui::Context::default();
    frame(&mut widget, &context, 0, None);
    // Public pointer/zoom input establishes zoom 0.35 at origin without adding
    // a profiling-only view setter to the widget's supported API.
    frame(&mut widget, &context, 1, Some(0.5));
    frame(&mut widget, &context, 2, Some(0.7));
    assert_eq!(widget.zoom_percent(), 35);
    println!(
        "Profiling {frames} idle CPU frames: 500 nodes / 2000 connections, zoom 0.35, viewport 1440x900"
    );
    for index in 0..frames {
        frame(&mut widget, &context, index + 3, None);
    }
    assert_eq!(serde_json::to_value(widget.graph()).unwrap(), before);
    assert_eq!(widget.zoom_percent(), 35);
    println!("Completed {frames} frames; graph unchanged.");
}
