//! egui response allocation and graph hit testing.
//!
//! This module owns per-frame response records and resolves screen positions
//! to opaque graph targets. It does not choose gestures, mutate graph state,
//! or define menu actions.

use std::collections::HashMap;

use egui::{Pos2, Rect};

use super::hit_target_moves::HitTargetMoves;
use super::layout::GraphWidgetLayout;
use super::minimap;
use super::widget::NodeGraphWidget;
use crate::model::{FrameId, NodeId, SocketId};

pub(crate) struct NodeResponses {
    pub(crate) body: egui::Response,
    pub(crate) header: egui::Response,
}

pub(crate) struct MinimapResponse {
    pub(crate) response: egui::Response,
    pub(crate) info: minimap::MinimapInfo,
}

pub(crate) struct GraphResponses {
    pub(crate) canvas: egui::Response,
    pub(crate) frames: HashMap<FrameId, egui::Response>,
    pub(crate) nodes: HashMap<NodeId, NodeResponses>,
    pub(crate) collapse_toggles: HashMap<NodeId, egui::Response>,
    pub(crate) sockets: HashMap<SocketId, egui::Response>,
    pub(crate) minimap: Option<MinimapResponse>,
}

pub(crate) enum ContextClickTarget {
    Canvas,
    Node(NodeId),
    Frame(FrameId),
}

// ── Hit-target ids ────────────────────────────────────────────────────────────
//
// Each id is registered twice per frame: once by `allocate_responses`, whose
// responses drive this frame's input, and once again while drawing, to lift a
// node's targets above the inline controls of the nodes painted behind it.

pub(crate) fn node_body_id(base: egui::Id, node: NodeId) -> egui::Id {
    base.with(("node-body", node.0))
}

pub(crate) fn node_header_id(base: egui::Id, node: NodeId) -> egui::Id {
    base.with(("node-header", node.0))
}

fn collapse_toggle_id(base: egui::Id, node: NodeId) -> egui::Id {
    base.with(("collapse-toggle", node.0))
}

fn socket_hit_id(base: egui::Id, socket: SocketId) -> egui::Id {
    base.with(("socket", socket.node.0, socket.index, socket.direction))
}

fn minimap_id(base: egui::Id) -> egui::Id {
    base.with("minimap")
}

fn raise(ui: &egui::Ui, rect: Rect, id: egui::Id, sense: egui::Sense) {
    refresh(ui, rect, id, sense, true);
}

fn refresh(ui: &egui::Ui, rect: Rect, id: egui::Id, sense: egui::Sense, move_to_top: bool) {
    // Offscreen targets retain their initial response registration. They cannot
    // cover a visible inline control, so moving them in egui's z-order is wasted
    // work. Test each target, not the node body: socket hit areas protrude beyond it.
    if !rect.intersects(ui.clip_rect()) {
        return;
    }
    ui.interact_opt(rect, id, sense, egui::InteractOptions { move_to_top });
}

impl GraphResponses {
    pub(crate) fn canvas_only(canvas: egui::Response) -> Self {
        Self {
            canvas,
            frames: HashMap::new(),
            nodes: HashMap::new(),
            collapse_toggles: HashMap::new(),
            sockets: HashMap::new(),
            minimap: None,
        }
    }
}

#[cfg(test)]
mod response_tests {
    use egui::{Event, Modifiers, PointerButton, Vec2};

    use super::*;
    use crate::model::SocketDirection;
    use crate::runtime::NodeTypeRegistry;

    struct OrderFixture {
        widget: NodeGraphWidget,
        layout: GraphWidgetLayout,
        context: egui::Context,
        base: egui::Id,
    }

    impl OrderFixture {
        fn new(zoom: f32, overlap: bool) -> Self {
            let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
            widget.view.zoom = zoom;
            widget.minimap_visible = false;
            let mut layout = widget.build_layout(Pos2::ZERO);
            for (id, x) in [
                (NodeId(1), 100.0),
                (NodeId(2), if overlap { 100.0 } else { 350.0 }),
            ] {
                let rect = |min: Pos2, size: Vec2| Rect::from_min_size(min * zoom, size * zoom);
                let body = rect(Pos2::new(x, 100.0), Vec2::new(120.0, 100.0));
                let header = rect(Pos2::new(x, 100.0), Vec2::new(120.0, 24.0));
                let toggle = rect(Pos2::new(x + 100.0, 104.0), Vec2::splat(12.0));
                layout.node_screen_rects.insert(id, body);
                layout.header_screen_rects.insert(id, header);
                layout.collapse_toggle_screen_rects.insert(id, toggle);
                let sockets: Vec<_> = [SocketDirection::Input, SocketDirection::Output]
                    .into_iter()
                    .map(|direction| SocketId {
                        node: id,
                        index: 0,
                        direction,
                    })
                    .collect();
                for &socket in &sockets {
                    layout
                        .socket_hit_rects
                        .insert(socket, rect(Pos2::new(x - 6.0, 142.0), Vec2::splat(12.0)));
                }
                // Deliberately explicit, independent of initial map iteration.
                layout.socket_hit_order_by_node.insert(id, sockets);
            }
            Self {
                widget,
                layout,
                context: egui::Context::default(),
                base: egui::Id::new("order-regression"),
            }
        }

        fn frame(
            &self,
            events: Vec<Event>,
            order: &[NodeId],
            overlay: Option<egui::Id>,
        ) -> HashMap<egui::Id, egui::Response> {
            self.frame_with_optimization(events, order, overlay, true)
        }

        fn frame_with_optimization(
            &self,
            events: Vec<Event>,
            order: &[NodeId],
            overlay: Option<egui::Id>,
            optimized: bool,
        ) -> HashMap<egui::Id, egui::Response> {
            let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(600.0));
            self.context.begin_pass(egui::RawInput {
                screen_rect: Some(clip),
                events,
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                self.context.clone(),
                self.base,
                egui::UiBuilder::new().max_rect(clip),
            );
            let canvas = ui.interact(
                clip,
                self.base.with("canvas"),
                egui::Sense::click_and_drag(),
            );
            let overlay_rect = self.layout.header_screen_rects[&NodeId(1)];
            // A minimap is initially registered before painting, then raised;
            // a floating panel is registered after all node targets.
            let mini = minimap_id(self.base);
            if overlay == Some(mini) {
                ui.interact(overlay_rect, mini, egui::Sense::click_and_drag());
            }
            self.widget
                .allocate_responses(&mut ui, canvas, &self.layout, clip);
            let moves = optimized.then(|| {
                HitTargetMoves::new(&ui, self.widget.view.zoom, &self.layout, &self.layout)
            });
            let mut ids = Vec::new();
            for &node in order {
                self.widget
                    .refresh_node_hit_targets(&ui, &self.layout, node, moves.as_ref());
                ids.extend([
                    node_body_id(self.base, node),
                    node_header_id(self.base, node),
                    collapse_toggle_id(self.base, node),
                ]);
                ids.extend(
                    self.layout.socket_hit_order_by_node[&node]
                        .iter()
                        .map(|&socket| socket_hit_id(self.base, socket)),
                );
            }
            if let Some(id) = overlay {
                if id == mini {
                    self.widget.raise_minimap_hit_target(&ui, overlay_rect);
                } else {
                    ui.interact(overlay_rect, id, egui::Sense::click_and_drag());
                }
                ids.push(id);
            }
            let responses = ids
                .into_iter()
                .map(|id| (id, self.context.read_response(id).unwrap()))
                .collect();
            let mut output = self.context.end_pass();
            output.textures_delta.clear();
            responses
        }
    }

    #[test]
    fn low_zoom_elision_requires_isolated_unchanged_unclipped_targets() {
        let fixture = OrderFixture::new(0.35, false);
        let node = NodeId(1);
        let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(600.0));
        fixture.context.begin_pass(egui::RawInput {
            screen_rect: Some(clip),
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            fixture.context.clone(),
            fixture.base,
            egui::UiBuilder::new().max_rect(clip),
        );
        let plan = |ui: &egui::Ui, zoom, current: &GraphWidgetLayout| {
            HitTargetMoves::new(ui, zoom, &fixture.layout, current)
        };
        let eligible = plan(&ui, 0.35, &fixture.layout);
        assert!(!eligible.base_move_to_top(node));
        assert!(
            fixture.layout.socket_hit_order_by_node[&node]
                .iter()
                .any(|&socket| !eligible.socket_move_to_top(socket))
        );
        for zoom in [0.6, 1.0, -1.0, f32::NAN] {
            assert!(plan(&ui, zoom, &fixture.layout).base_move_to_top(node));
        }
        ui.set_clip_rect(fixture.layout.node_screen_rects[&node]);
        assert!(plan(&ui, 0.35, &fixture.layout).base_move_to_top(node));
        ui.set_clip_rect(clip);
        fixture.context.set_transform_layer(
            ui.layer_id(),
            egui::emath::TSTransform::from_translation(Vec2::new(1.0, 0.0)),
        );
        assert!(plan(&ui, 0.35, &fixture.layout).base_move_to_top(node));
        fixture
            .context
            .set_transform_layer(ui.layer_id(), egui::emath::TSTransform::IDENTITY);
        for radius in [-1.0, f32::NAN, f32::INFINITY] {
            fixture
                .context
                .global_style_mut(|style| style.interaction.interact_radius = radius);
            assert!(plan(&ui, 0.35, &fixture.layout).base_move_to_top(node));
        }
        fixture
            .context
            .global_style_mut(|style| style.interaction.interact_radius = 5.0);
        assert!(!plan(&ui, 0.35, &fixture.layout).base_move_to_top(node));
        for change in 0..8 {
            let mut current = fixture.widget.build_layout(Pos2::ZERO);
            current.node_screen_rects = fixture.layout.node_screen_rects.clone();
            current.header_screen_rects = fixture.layout.header_screen_rects.clone();
            current.collapse_toggle_screen_rects =
                fixture.layout.collapse_toggle_screen_rects.clone();
            current.socket_hit_rects = fixture.layout.socket_hit_rects.clone();
            current.socket_hit_order_by_node = fixture.layout.socket_hit_order_by_node.clone();
            let socket = current.socket_hit_order_by_node[&node][0];
            match change {
                0 => current.node_screen_rects.get_mut(&node).unwrap().max.x += 1.0,
                1 => current.header_screen_rects.get_mut(&node).unwrap().max.x += 1.0,
                2 => {
                    current
                        .collapse_toggle_screen_rects
                        .get_mut(&node)
                        .unwrap()
                        .max
                        .x += 1.0
                }
                3 => current.socket_hit_rects.get_mut(&socket).unwrap().max.x += 1.0,
                4 => {
                    current.socket_hit_rects.remove(&socket);
                    current
                        .socket_hit_order_by_node
                        .get_mut(&node)
                        .unwrap()
                        .remove(0);
                }
                5 => {
                    current
                        .node_screen_rects
                        .insert(NodeId(2), current.node_screen_rects[&node]);
                }
                6 => {
                    // A deleted node's old targets remain registered this pass.
                    current.node_screen_rects.remove(&NodeId(2));
                    current.header_screen_rects.remove(&NodeId(2));
                    current.collapse_toggle_screen_rects.remove(&NodeId(2));
                    current
                        .socket_hit_rects
                        .retain(|socket, _| socket.node != NodeId(2));
                    current.socket_hit_order_by_node.remove(&NodeId(2));
                }
                _ => {
                    // Equal counts do not imply identical targets: replace a
                    // socket on the other node, leaving this node unchanged.
                    let removed = current.socket_hit_order_by_node[&NodeId(2)][0];
                    let added = SocketId {
                        index: 1,
                        ..removed
                    };
                    let rect = current.socket_hit_rects.remove(&removed).unwrap();
                    current.socket_hit_rects.insert(added, rect);
                    current
                        .socket_hit_order_by_node
                        .get_mut(&NodeId(2))
                        .unwrap()[0] = added;
                }
            }
            assert!(
                plan(&ui, 0.35, &current).base_move_to_top(node),
                "change {change}"
            );
        }
        fixture.context.end_pass().textures_delta.clear();
    }

    #[test]
    fn low_zoom_elision_matches_full_raising_at_direct_and_near_hits() {
        for overlap in [false, true] {
            let mut fixture = OrderFixture::new(0.35, overlap);
            let order = [NodeId(1), NodeId(2)];
            for x in (80..240).step_by(12) {
                for y in (80..230).step_by(12) {
                    let pointer = Pos2::new(x as f32, y as f32) * 0.35;
                    let mut winners = Vec::new();
                    for optimized in [false, true] {
                        fixture.context = egui::Context::default();
                        for _ in 0..2 {
                            let responses = fixture.frame_with_optimization(
                                vec![Event::PointerMoved(pointer)],
                                &order,
                                None,
                                optimized,
                            );
                            winners.push(
                                responses
                                    .into_iter()
                                    .filter_map(|(id, r)| r.hovered().then_some(id))
                                    .collect::<std::collections::HashSet<_>>(),
                            );
                        }
                    }
                    assert_eq!(winners[1], winners[3], "{pointer:?}, overlap {overlap}");
                }
            }
        }
    }

    #[test]
    fn low_zoom_overlap_and_floating_targets_follow_paint_order() {
        for zoom in [0.35, 1.0] {
            for order in [[NodeId(1), NodeId(2)], [NodeId(2), NodeId(1)]] {
                let fixture = OrderFixture::new(zoom, true);
                for overlay in [
                    None,
                    Some(minimap_id(fixture.base)),
                    Some(fixture.base.with("panel")),
                ] {
                    let pointer = Pos2::new(150.0, 112.0) * zoom;
                    fixture.frame(vec![Event::PointerMoved(pointer)], &order, overlay);
                    let responses =
                        fixture.frame(vec![Event::PointerMoved(pointer)], &order, overlay);
                    let expected =
                        overlay.unwrap_or_else(|| node_header_id(fixture.base, order[1]));
                    assert!(
                        responses[&expected].hovered(),
                        "zoom {zoom}, order {order:?}, overlay {overlay:?}"
                    );
                    for (id, response) in responses {
                        if id != expected {
                            assert!(!response.hovered(), "covered target {id:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn socket_raise_order_overrides_initial_toggle_and_socket_order() {
        let mut fixture = OrderFixture::new(0.35, false);
        let node = NodeId(1);
        let rect = fixture.layout.collapse_toggle_screen_rects[&node];
        for &socket in &fixture.layout.socket_hit_order_by_node[&node] {
            fixture.layout.socket_hit_rects.insert(socket, rect);
        }
        for _ in 0..2 {
            fixture
                .layout
                .socket_hit_order_by_node
                .get_mut(&node)
                .unwrap()
                .reverse();
            let expected = socket_hit_id(
                fixture.base,
                *fixture.layout.socket_hit_order_by_node[&node]
                    .last()
                    .unwrap(),
            );
            fixture.frame(vec![Event::PointerMoved(rect.center())], &[node], None);
            let responses = fixture.frame(vec![Event::PointerMoved(rect.center())], &[node], None);
            assert!(responses[&expected].hovered());
            assert!(!responses[&collapse_toggle_id(fixture.base, node)].hovered());
            assert_eq!(
                responses
                    .values()
                    .filter(|response| response.hovered())
                    .count(),
                1
            );
        }
    }

    #[test]
    fn target_refresh_preserves_tab_focus_and_pointer_capture_after_geometry_changes() {
        let mut fixture = OrderFixture::new(0.35, false);
        let order = [NodeId(1), NodeId(2)];
        fixture.frame(Vec::new(), &order, None);
        let body = node_body_id(fixture.base, NodeId(1));
        let header = node_header_id(fixture.base, NodeId(1));
        fixture
            .context
            .memory_mut(|memory| memory.request_focus(body));
        fixture.frame(Vec::new(), &order, None);
        let responses = fixture.frame(
            vec![Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            &order,
            None,
        );
        assert!(
            responses[&header].has_focus(),
            "body and header retain their initial focus-registration order"
        );
        let responses = fixture.frame(Vec::new(), &order, None);
        assert!(
            responses[&header].has_focus(),
            "raising must not surrender keyboard focus"
        );

        // Crossing the inline-control zoom threshold switches between elided
        // and full raising without changing target identity or focus order.
        for zoom in [1.0, 0.35] {
            let scale = zoom / fixture.widget.view.zoom;
            for rect in fixture
                .layout
                .node_screen_rects
                .values_mut()
                .chain(fixture.layout.header_screen_rects.values_mut())
                .chain(fixture.layout.collapse_toggle_screen_rects.values_mut())
                .chain(fixture.layout.socket_hit_rects.values_mut())
            {
                *rect = Rect::from_min_max(rect.min * scale, rect.max * scale);
            }
            fixture.widget.view.zoom = zoom;
            let responses = fixture.frame(Vec::new(), &order, None);
            assert!(responses[&header].has_focus(), "zoom {zoom}");
            assert_eq!(
                responses[&header].rect,
                fixture.layout.header_screen_rects[&NodeId(1)]
            );
        }

        let start = fixture.layout.header_screen_rects[&NodeId(1)].center();
        fixture.frame(vec![Event::PointerMoved(start)], &order, None);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let responses = fixture.frame(vec![button(start, true)], &order, None);
        assert!(responses[&header].is_pointer_button_down_on());
        let outside = Pos2::new(700.0, 700.0);
        // Input can change geometry between passes while pointer capture stays
        // with the stable target id, even when the pointer leaves the graph.
        let moved = fixture.layout.header_screen_rects[&NodeId(1)].translate(Vec2::new(30.0, 30.0));
        fixture.layout.header_screen_rects.insert(NodeId(1), moved);
        let responses = fixture.frame(vec![Event::PointerMoved(outside)], &order, None);
        assert_eq!(responses[&header].rect, moved);
        assert!(responses[&header].dragged());
        assert!(!responses[&node_header_id(fixture.base, NodeId(2))].dragged());
        let responses = fixture.frame(vec![button(outside, false)], &order, None);
        assert!(responses[&header].drag_stopped());
    }

    #[test]
    fn clipped_targets_keep_their_place_in_keyboard_focus_order() {
        for zoom in [0.35, 1.0] {
            let mut fixture = OrderFixture::new(zoom, false);
            // Initial response allocation follows this map's iteration order.
            // Preserve it: clipping does not remove egui's focus interest, so
            // partitioning registration into hidden/visible groups changes Tab.
            let nodes: Vec<_> = fixture.layout.node_screen_rects.keys().copied().collect();
            let visible = nodes[0];
            let clipped = nodes[1];
            for rects in [
                &mut fixture.layout.node_screen_rects,
                &mut fixture.layout.header_screen_rects,
                &mut fixture.layout.collapse_toggle_screen_rects,
            ] {
                let rect = rects.get_mut(&clipped).unwrap();
                *rect = rect.translate(Vec2::new(800.0, 0.0));
            }
            for (socket, rect) in &mut fixture.layout.socket_hit_rects {
                if socket.node == clipped {
                    *rect = rect.translate(Vec2::new(800.0, 0.0));
                }
            }
            fixture.frame(Vec::new(), &nodes, None);
            let header = node_header_id(fixture.base, visible);
            let body = node_body_id(fixture.base, clipped);
            fixture
                .context
                .memory_mut(|memory| memory.request_focus(header));
            fixture.frame(Vec::new(), &nodes, None);
            let tab = |modifiers| Event::Key {
                key: egui::Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            let responses = fixture.frame(vec![tab(Modifiers::NONE)], &nodes, None);
            assert!(
                responses[&body].has_focus(),
                "Tab must reach the clipped target"
            );
            assert!(!responses[&body].interact_rect.is_positive());
            fixture.frame(vec![tab(Modifiers::SHIFT)], &nodes, None);
            let responses = fixture.frame(Vec::new(), &nodes, None);
            assert!(
                responses[&header].has_focus(),
                "Shift-Tab must return to the visible header"
            );
        }
    }

    #[test]
    fn captured_target_stays_live_when_its_geometry_becomes_fully_clipped() {
        let mut fixture = OrderFixture::new(0.35, false);
        let node = NodeId(1);
        let order = [node, NodeId(2)];
        let header = node_header_id(fixture.base, node);
        let start = fixture.layout.header_screen_rects[&node].center();
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        fixture.frame(vec![Event::PointerMoved(start)], &order, None);
        let responses = fixture.frame(vec![button(start, true)], &order, None);
        assert!(responses[&header].is_pointer_button_down_on());
        let moved = fixture.layout.header_screen_rects[&node].translate(Vec2::new(800.0, 0.0));
        fixture.layout.header_screen_rects.insert(node, moved);
        let outside = moved.center();
        let responses = fixture.frame(vec![Event::PointerMoved(outside)], &order, None);
        assert_eq!(responses[&header].rect, moved);
        assert!(!responses[&header].interact_rect.is_positive());
        assert!(responses[&header].dragged());
        assert!(responses[&header].is_pointer_button_down_on());
        let responses = fixture.frame(vec![button(outside, false)], &order, None);
        assert!(responses[&header].drag_stopped());
        assert!(!responses[&header].is_pointer_button_down_on());
    }

    #[test]
    fn indexed_raising_preserves_the_flat_order_winner_for_overlapping_sockets() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let node = widget
            .add_node_at("Reroute", Pos2::new(100.0, 100.0))
            .unwrap();
        let mut layout = widget.build_layout(Pos2::ZERO);
        let hit = Rect::from_center_size(Pos2::new(112.0, 112.0), Vec2::splat(12.0));
        for rect in layout.socket_hit_rects.values_mut() {
            *rect = hit;
        }
        let flat: Vec<_> = layout.socket_hit_rects.keys().copied().collect();
        assert_eq!(flat, layout.socket_hit_order_by_node[&node]);
        let expected = *flat.last().unwrap();
        let context = egui::Context::default();
        let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(300.0));
        let mut hovered = false;
        for _ in 0..2 {
            context.begin_pass(egui::RawInput {
                screen_rect: Some(clip),
                events: vec![egui::Event::PointerMoved(hit.center())],
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                context.clone(),
                egui::Id::new("indexed-overlap"),
                egui::UiBuilder::new().max_rect(clip),
            );
            let canvas = ui.interact(clip, ui.id().with("canvas"), egui::Sense::click_and_drag());
            widget.allocate_responses(&mut ui, canvas, &layout, clip);
            widget.raise_node_hit_targets(&ui, &layout, node);
            hovered = context
                .read_response(socket_hit_id(ui.id(), expected))
                .unwrap()
                .hovered();
            let mut output = context.end_pass();
            output.textures_delta.clear();
        }
        assert!(hovered);
    }

    #[test]
    fn clipped_raise_keeps_registration_and_resumes_when_target_enters_clip() {
        let context = egui::Context::default();
        let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0));
        context.begin_pass(egui::RawInput {
            screen_rect: Some(clip),
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("clipped-raise"),
            egui::UiBuilder::new().max_rect(clip),
        );
        ui.set_clip_rect(clip);
        let id = ui.id().with("target");
        let original = Rect::from_min_size(Pos2::new(300.0, 100.0), Vec2::splat(20.0));
        ui.interact(original, id, egui::Sense::click_and_drag());
        raise(
            &ui,
            original.translate(Vec2::new(20.0, 0.0)),
            id,
            egui::Sense::click_and_drag(),
        );
        assert_eq!(
            context.read_response(id).unwrap().rect,
            original,
            "offscreen raising must not update/reorder the initial registration"
        );
        let visible = original.translate(Vec2::new(-150.0, 0.0));
        raise(&ui, visible, id, egui::Sense::click_and_drag());
        assert_eq!(context.read_response(id).unwrap().rect, visible);
        let mut output = context.end_pass();
        output.textures_delta.clear();
    }

    #[test]
    fn protruding_socket_is_raised_even_when_its_node_body_is_offscreen() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let node = widget
            .add_node_at("Reroute", Pos2::new(201.0, 100.0))
            .unwrap();
        let clip = Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0));
        let layout = widget.build_layout(Pos2::ZERO);
        let socket = SocketId {
            node,
            index: 0,
            direction: SocketDirection::Input,
        };
        assert!(!layout.node_screen_rects[&node].intersects(clip));
        assert!(layout.socket_hit_rects[&socket].intersects(clip));
        let pointer = Pos2::new(199.0, 112.0);
        let context = egui::Context::default();
        let mut hovered = None;
        for _ in 0..2 {
            context.begin_pass(egui::RawInput {
                screen_rect: Some(clip),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                context.clone(),
                egui::Id::new("protruding-target"),
                egui::UiBuilder::new().max_rect(clip),
            );
            ui.set_clip_rect(clip);
            let canvas = ui.interact(clip, ui.id().with("canvas"), egui::Sense::click_and_drag());
            widget.allocate_responses(&mut ui, canvas, &layout, clip);
            let covered = ui.id().with("covered-control");
            ui.interact(
                Rect::from_min_max(Pos2::new(190.0, 100.0), Pos2::new(200.0, 125.0)),
                covered,
                egui::Sense::click_and_drag(),
            );
            widget.raise_node_hit_targets(&ui, &layout, node);
            hovered = Some((
                context
                    .read_response(socket_hit_id(ui.id(), socket))
                    .unwrap()
                    .hovered(),
                context.read_response(covered).unwrap().hovered(),
            ));
            let mut output = context.end_pass();
            output.textures_delta.clear();
        }
        assert_eq!(hovered, Some((true, false)));
    }
}

impl NodeGraphWidget {
    pub(crate) fn allocate_responses(
        &self,
        ui: &mut egui::Ui,
        canvas_response: egui::Response,
        layout: &GraphWidgetLayout,
        canvas_rect: Rect,
    ) -> GraphResponses {
        let frames = layout
            .frame_screen_rects
            .iter()
            .map(|(&id, &rect)| {
                (
                    id,
                    ui.interact(
                        rect,
                        ui.id().with(("frame", id.0)),
                        egui::Sense::click_and_drag(),
                    ),
                )
            })
            .collect();

        let mut nodes = HashMap::new();
        for (&id, &body_rect) in &layout.node_screen_rects {
            let Some(&header_rect) = layout.header_screen_rects.get(&id) else {
                continue;
            };
            // Embedded controls are drawn later in the frame, so they sit on
            // top of this region and still receive their own clicks/drags.
            // `refresh_node_hit_targets` re-registers these while drawing so
            // that only the *own* node's controls end up above them.
            let body = ui.interact(
                body_rect,
                node_body_id(ui.id(), id),
                egui::Sense::click_and_drag(),
            );
            let header = ui.interact(
                header_rect,
                node_header_id(ui.id(), id),
                egui::Sense::click_and_drag(),
            );
            nodes.insert(id, NodeResponses { body, header });
        }

        let sockets = layout
            .socket_hit_rects
            .iter()
            .map(|(&socket_id, &rect)| {
                (
                    socket_id,
                    ui.interact(
                        rect,
                        socket_hit_id(ui.id(), socket_id),
                        egui::Sense::click_and_drag(),
                    ),
                )
            })
            .collect();
        let collapse_toggles = layout
            .collapse_toggle_screen_rects
            .iter()
            .map(|(&node_id, &rect)| {
                (
                    node_id,
                    ui.interact(
                        rect,
                        collapse_toggle_id(ui.id(), node_id),
                        egui::Sense::click(),
                    ),
                )
            })
            .collect();

        let minimap = self.minimap_visible.then(|| {
            let (info, rect) =
                minimap::compute_minimap(layout.node_rects.values().copied(), canvas_rect);
            let response = ui.interact(rect, minimap_id(ui.id()), egui::Sense::click_and_drag());
            MinimapResponse { response, info }
        });

        GraphResponses {
            canvas: canvas_response,
            frames,
            nodes,
            collapse_toggles,
            sockets,
            minimap,
        }
    }

    /// Re-registers one node's hit targets above every widget registered so
    /// far this frame.
    ///
    /// egui resolves overlapping widgets inside a layer by registration
    /// order — last one wins — and a node's inline controls are real widgets
    /// registered while drawing, i.e. after every hit target allocated by
    /// `allocate_responses`. Left at that, a control on a node painted
    /// *behind* another node keeps stealing hover and clicks from the node
    /// covering it: a text field lighting up while the pointer is on the
    /// header of the node in front of it, and refusing to let that header be
    /// dragged. Calling this per node in painting order restores the painted
    /// z-order for interaction too: node, then its own controls, then the
    /// next node on top of both.
    #[cfg(test)]
    pub(crate) fn raise_node_hit_targets(
        &self,
        ui: &egui::Ui,
        layout: &GraphWidgetLayout,
        node_id: NodeId,
    ) {
        self.refresh_node_hit_targets(ui, layout, node_id, None);
    }

    pub(crate) fn refresh_node_hit_targets(
        &self,
        ui: &egui::Ui,
        layout: &GraphWidgetLayout,
        node_id: NodeId,
        moves: Option<&HitTargetMoves>,
    ) {
        let base_move = moves.is_none_or(|moves| moves.base_move_to_top(node_id));
        if let Some(&rect) = layout.node_screen_rects.get(&node_id) {
            refresh(
                ui,
                rect,
                node_body_id(ui.id(), node_id),
                egui::Sense::click_and_drag(),
                base_move,
            );
        }
        if let Some(&rect) = layout.header_screen_rects.get(&node_id) {
            refresh(
                ui,
                rect,
                node_header_id(ui.id(), node_id),
                egui::Sense::click_and_drag(),
                base_move,
            );
        }
        if let Some(&rect) = layout.collapse_toggle_screen_rects.get(&node_id) {
            refresh(
                ui,
                rect,
                collapse_toggle_id(ui.id(), node_id),
                egui::Sense::click(),
                base_move,
            );
        }
        if let Some(sockets) = layout.socket_hit_order_by_node.get(&node_id) {
            for &socket_id in sockets {
                refresh(
                    ui,
                    layout.socket_hit_rects[&socket_id],
                    socket_hit_id(ui.id(), socket_id),
                    egui::Sense::click_and_drag(),
                    moves.is_none_or(|moves| moves.socket_move_to_top(socket_id)),
                );
            }
        }
    }

    /// Keeps the minimap above the node hit targets raised during drawing —
    /// it floats over the canvas, so nodes underneath must not claim its
    /// clicks and drags.
    pub(crate) fn raise_minimap_hit_target(&self, ui: &egui::Ui, rect: Rect) {
        raise(ui, rect, minimap_id(ui.id()), egui::Sense::click_and_drag());
    }

    /// Painting order key: nodes are drawn by ascending id, with the node
    /// most recently raised drawn last. Overlap resolution — both egui's and
    /// this module's own hit testing — has to agree with what the user sees.
    fn node_paint_order(&self, node_id: NodeId) -> (bool, u32) {
        (self.top_node == Some(node_id), node_id.0)
    }

    pub(crate) fn node_at_screen_pos(
        &self,
        responses: &GraphResponses,
        screen_pos: Pos2,
    ) -> Option<NodeId> {
        let hits_node = |&id: &NodeId| {
            responses
                .collapse_toggles
                .get(&id)
                .is_some_and(|response| response.rect.contains(screen_pos))
                || responses.nodes.get(&id).is_some_and(|node| {
                    node.header.rect.contains(screen_pos) || node.body.rect.contains(screen_pos)
                })
                || responses.sockets.iter().any(|(socket_id, response)| {
                    socket_id.node == id && response.rect.contains(screen_pos)
                })
        };
        responses
            .nodes
            .keys()
            .copied()
            .filter(|id| hits_node(id))
            .max_by_key(|&id| self.node_paint_order(id))
    }

    pub(crate) fn frame_at_screen_pos(
        &self,
        responses: &GraphResponses,
        layout: &GraphWidgetLayout,
        screen_pos: Pos2,
    ) -> Option<FrameId> {
        responses
            .frames
            .keys()
            .filter(|id| {
                layout
                    .frame_screen_rects
                    .get(id)
                    .is_some_and(|rect| rect.contains(screen_pos))
            })
            .min_by(|a, b| {
                let a_rect = layout.frame_screen_rects[a];
                let b_rect = layout.frame_screen_rects[b];
                a_rect
                    .area()
                    .total_cmp(&b_rect.area())
                    .then_with(|| a.0.cmp(&b.0))
            })
            .copied()
    }

    pub(crate) fn context_click_target_at(
        &self,
        responses: &GraphResponses,
        layout: &GraphWidgetLayout,
        screen_pos: Pos2,
    ) -> Option<ContextClickTarget> {
        if let Some(id) = self.node_at_screen_pos(responses, screen_pos) {
            return Some(ContextClickTarget::Node(id));
        }
        if let Some(id) = self.frame_at_screen_pos(responses, layout, screen_pos) {
            return Some(ContextClickTarget::Frame(id));
        }
        responses
            .canvas
            .rect
            .contains(screen_pos)
            .then_some(ContextClickTarget::Canvas)
    }
}
