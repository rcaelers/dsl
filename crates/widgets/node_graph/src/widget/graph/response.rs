//! egui response allocation and graph hit testing.
//!
//! This module owns per-frame response records and resolves screen positions
//! to opaque graph targets. It does not choose gestures, mutate graph state,
//! or define menu actions.

use std::collections::HashMap;

use egui::{Pos2, Rect};

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
    // Offscreen targets retain their initial response registration. They cannot
    // cover a visible inline control, so moving them in egui's z-order is wasted
    // work. Test each target, not the node body: socket hit areas protrude beyond it.
    if !rect.intersects(ui.clip_rect()) {
        return;
    }
    ui.interact_opt(rect, id, sense, egui::InteractOptions { move_to_top: true });
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
    use egui::Vec2;

    use super::*;
    use crate::model::SocketDirection;
    use crate::runtime::NodeTypeRegistry;

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
            // `raise_node_hit_targets` re-registers these while drawing so
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
    pub(crate) fn raise_node_hit_targets(
        &self,
        ui: &egui::Ui,
        layout: &GraphWidgetLayout,
        node_id: NodeId,
    ) {
        if let Some(&rect) = layout.node_screen_rects.get(&node_id) {
            raise(
                ui,
                rect,
                node_body_id(ui.id(), node_id),
                egui::Sense::click_and_drag(),
            );
        }
        if let Some(&rect) = layout.header_screen_rects.get(&node_id) {
            raise(
                ui,
                rect,
                node_header_id(ui.id(), node_id),
                egui::Sense::click_and_drag(),
            );
        }
        if let Some(&rect) = layout.collapse_toggle_screen_rects.get(&node_id) {
            raise(
                ui,
                rect,
                collapse_toggle_id(ui.id(), node_id),
                egui::Sense::click(),
            );
        }
        for (&socket_id, &rect) in &layout.socket_hit_rects {
            if socket_id.node == node_id {
                raise(
                    ui,
                    rect,
                    socket_hit_id(ui.id(), socket_id),
                    egui::Sense::click_and_drag(),
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
