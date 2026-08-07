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
            let body = ui.interact(
                body_rect,
                ui.id().with(("node-body", id.0)),
                egui::Sense::click_and_drag(),
            );
            let header = ui.interact(
                header_rect,
                ui.id().with(("node-header", id.0)),
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
                        ui.id().with((
                            "socket",
                            socket_id.node.0,
                            socket_id.index,
                            socket_id.direction,
                        )),
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
                        ui.id().with(("collapse-toggle", node_id.0)),
                        egui::Sense::click(),
                    ),
                )
            })
            .collect();

        let minimap = self.minimap_visible.then(|| {
            let (info, rect) =
                minimap::compute_minimap(layout.node_rects.values().copied(), canvas_rect);
            let response =
                ui.interact(rect, ui.id().with("minimap"), egui::Sense::click_and_drag());
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

    pub(crate) fn node_at_screen_pos(
        &self,
        responses: &GraphResponses,
        screen_pos: Pos2,
    ) -> Option<NodeId> {
        if let Some((&id, _)) = responses
            .collapse_toggles
            .iter()
            .find(|(_, response)| response.rect.contains(screen_pos))
        {
            return Some(id);
        }
        if let Some((&id, _)) = responses.nodes.iter().find(|(_, node)| {
            node.header.rect.contains(screen_pos) || node.body.rect.contains(screen_pos)
        }) {
            return Some(id);
        }
        responses
            .sockets
            .iter()
            .find_map(|(&id, response)| response.rect.contains(screen_pos).then_some(id.node))
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
