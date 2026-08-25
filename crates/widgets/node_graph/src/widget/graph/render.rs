use egui::{Color32, CornerRadius, Painter, Pos2, Rect, RichText, Stroke, Vec2};

use super::interaction_state::InteractionState;
use super::layout::GraphWidgetLayout;
use super::minimap;
use super::response::GraphResponses;
use super::widget::NodeGraphWidget;
use crate::model::{NodeId, Socket, SocketDirection, SocketId};
use crate::support::{
    SOCKET_RADIUS, WireEmphasis, draw_box_select, draw_connections, draw_frames, draw_grid,
    draw_knife_line, draw_wire, egui_color, to_screen_rect,
};
use crate::widget::node::{NodeControlContext, NodeDrawContext};

pub(crate) struct GraphRenderContext<'a> {
    pub rect: Rect,
    pub origin: Pos2,
    pub pointer: Option<Pos2>,
    pub layout: &'a GraphWidgetLayout,
    pub hovered_socket: Option<SocketId>,
}

impl NodeGraphWidget {
    pub(crate) fn draw_graph(
        &mut self,
        ui: &mut egui::Ui,
        painter: &Painter,
        context: GraphRenderContext<'_>,
    ) {
        let GraphRenderContext {
            rect,
            origin,
            pointer,
            layout,
            hovered_socket,
        } = context;
        // `painter` is already clipped to `rect`, but the inline node
        // controls below (`show_controls`) place real egui widgets straight
        // on `ui` at absolute screen positions — unclipped, those bleed
        // outside the graph's own area (e.g. into a sibling panel above)
        // once a node is panned off past the canvas edge.
        let previous_clip = ui.clip_rect();
        ui.set_clip_rect(rect.intersect(previous_clip));

        let pointer_canvas = pointer.map(|p| self.view.screen_to_canvas(origin, p));

        // While a node is dragged over a wire, that wire previews the drop:
        // highlighted when the node can be spliced in, muted when it can't.
        let insert_candidate =
            if let InteractionState::DraggingNode { node_id, .. } = self.interaction_state {
                self.compute_insert_candidate_wire(node_id, pointer_canvas, &layout.nodes)
            } else {
                None
            };
        // Likewise, the frame a dragged node would join on release gets a
        // brighter outline so the drop target is obvious before release
        // (Phase 1.3). A node already in a frame is never a candidate here —
        // dragging can only add a node to a frame, never remove it.
        let drop_target_frame =
            if let InteractionState::DraggingNode { node_id, .. } = self.interaction_state {
                self.compute_drop_target_frame(node_id, layout)
            } else {
                None
            };

        draw_grid(painter, rect, &self.view);
        draw_frames(
            painter,
            &self.graph,
            &layout.frame_rects,
            &self.view,
            origin,
        );
        if let Some(frame_id) = drop_target_frame
            && let Some(&screen_rect) = layout.frame_screen_rects.get(&frame_id)
        {
            painter.rect_stroke(
                screen_rect,
                egui::CornerRadius::same(6),
                Stroke::new(2.5, Color32::WHITE),
                egui::StrokeKind::Middle,
            );
        }

        let wire_w = (2.0 * self.view.zoom).clamp(1.0_f32, 4.0_f32);
        draw_connections(
            painter,
            &self.graph,
            &self.registry,
            &layout.socket_screen_pos,
            wire_w,
            |idx, conn| match insert_candidate {
                Some((candidate, insertable)) if candidate == idx => {
                    if insertable {
                        WireEmphasis::Highlight
                    } else {
                        WireEmphasis::Muted
                    }
                }
                _ if hovered_socket
                    .is_some_and(|socket| conn.from == socket || conn.to == socket) =>
                {
                    WireEmphasis::Highlight
                }
                _ => {
                    let endpoint_selected = [conn.from.node, conn.to.node]
                        .iter()
                        .any(|id| self.graph.nodes.get(id).is_some_and(|n| n.selected));
                    if endpoint_selected {
                        WireEmphasis::Highlight
                    } else {
                        WireEmphasis::Normal
                    }
                }
            },
        );
        let mut socket_highlights = Vec::new();
        if let Some(socket_id) = hovered_socket {
            for socket_id in self.socket_highlight_cluster(socket_id) {
                if let Some(&pos) = layout.socket_screen_pos.get(&socket_id) {
                    let color = self.socket_display_color(socket_id);
                    socket_highlights.push((self.view.screen_to_canvas(origin, pos), color));
                }
            }
        }

        // Owns a clone of the `Rc` (cheap) rather than borrowing
        // `self.interaction_state` — the dim-drawing loop below also calls
        // `self.run_update`, which needs `&mut self`.
        let mut wire_drag_dim: Option<(NodeId, std::rc::Rc<std::collections::HashSet<NodeId>>)> =
            None;
        if let InteractionState::DraggingWire {
            from,
            from_canvas,
            current_canvas,
            connectable,
            ..
        } = &self.interaction_state
        {
            wire_drag_dim = Some((from.node, connectable.clone()));
            let snap = self.snapped_wire_target(*from, *current_canvas, layout);
            let preview_end_canvas = snap.map_or(*current_canvas, |(_, pos)| pos);
            let color = self
                .graph
                .nodes
                .get(&from.node)
                .and_then(|n| {
                    if from.direction == SocketDirection::Output {
                        n.outputs.get(from.index).map(|s| egui_color(s.color))
                    } else {
                        n.inputs.get(from.index).map(|s| egui_color(s.color))
                    }
                })
                .unwrap_or(Color32::from_rgb(160, 160, 160));
            socket_highlights.push((*from_canvas, color));
            // A wire is shaped output-first: it leaves the first point
            // towards the right and enters the second from the left. The
            // anchored end of this drag is an input whenever a link is being
            // moved or a wire is pulled off an input socket, and there the
            // free end plays the output — passing the ends the other way
            // round is what keeps the preview from looping back on itself.
            let anchor_screen = self.view.canvas_to_screen(origin, *from_canvas);
            let free_screen = self.view.canvas_to_screen(origin, preview_end_canvas);
            let (wire_start, wire_end) = if from.direction == SocketDirection::Output {
                (anchor_screen, free_screen)
            } else {
                (free_screen, anchor_screen)
            };
            draw_wire(painter, wire_start, wire_end, color, wire_w);
            if let Some((target, target_canvas)) = snap {
                let target_socket =
                    self.graph
                        .nodes
                        .get(&target.node)
                        .and_then(|node| match target.direction {
                            SocketDirection::Input => node.inputs.get(target.index),
                            SocketDirection::Output => node.outputs.get(target.index),
                        });
                // Preview what a multi-accepting input would resolve to: the
                // dragged output's type identity instead of the idle look.
                let from_type = self
                    .graph
                    .nodes
                    .get(&from.node)
                    .filter(|_| from.direction == SocketDirection::Output)
                    .and_then(|n| n.outputs.get(from.index))
                    .map(|s| s.effective_type().to_owned());
                let highlight = match (target_socket, from_type) {
                    (Some(socket), Some(ft))
                        if target.direction == SocketDirection::Input
                            && ft != "Any"
                            && ft != socket.type_name =>
                    {
                        self.registry
                            .socket_type_style(&ft)
                            .map(|style| style.color)
                            .unwrap_or_else(|| egui_color(socket.color))
                    }
                    (Some(socket), _) => egui_color(socket.color),
                    (None, _) => color,
                };
                socket_highlights.push((target_canvas, highlight));
            }
        }

        if let InteractionState::DraggingNode { node_id, .. } = self.interaction_state {
            self.top_node = Some(node_id);
        }
        let mut sorted = self.graph.sorted_node_ids();
        if let Some(top) = self.top_node {
            if sorted.contains(&top) {
                sorted.retain(|id| *id != top);
                sorted.push(top);
            } else {
                self.top_node = None;
            }
        }

        for id in sorted {
            if let (Some(widget), Some(node)) = (layout.nodes.get(&id), self.graph.nodes.get(&id)) {
                let badge = self.external_badges.get(&id).or(node.badge.as_ref());
                let status = self.node_statuses.get(&id).map(String::as_str);
                widget.draw(
                    painter,
                    id,
                    node,
                    NodeDrawContext {
                        graph: &self.graph,
                        badge,
                        status,
                        registry: &self.registry,
                        view: &self.view,
                        origin,
                        socket_indicators: &self.socket_indicators,
                    },
                );
                if let Some((source_id, connectable)) = &wire_drag_dim
                    && id != *source_id
                    && !connectable.contains(&id)
                {
                    // Widened past the node rect on the left/right — input
                    // and output socket centers sit exactly on those edges,
                    // so half of each socket shape draws outside it and
                    // would otherwise stay bright while the rest dims.
                    let bulge = self.view.scale(SOCKET_RADIUS * 1.3);
                    let screen_rect = to_screen_rect(widget.node_rect(), &self.view, origin)
                        .expand2(Vec2::new(bulge, 0.0));
                    painter.rect_filled(
                        screen_rect,
                        CornerRadius::same(5),
                        Color32::from_rgba_unmultiplied(28, 28, 28, 153),
                    );
                }
            }
            // Interaction z-order has to follow the painting order above:
            // this node covers the inline controls of every node behind it,
            // so its hit targets are lifted over them before its own
            // controls register themselves on top.
            if !self.interaction_state.use_fast_rendering() {
                self.raise_node_hit_targets(ui, layout, id);
            }
            if self.view.zoom >= 0.6 {
                let changed = if let (Some(widget), Some(node), Some(instance)) = (
                    layout.nodes.get(&id),
                    self.graph.nodes.get(&id),
                    self.runtime.get_mut(&id),
                ) {
                    ui.add_enabled_ui(self.editing_enabled, |ui| {
                        widget.show_controls(
                            ui,
                            id,
                            node,
                            instance.as_mut(),
                            NodeControlContext {
                                graph: &self.graph,
                                view: &self.view,
                                origin,
                                file_dialog: self.file_dialog_service.as_mut(),
                            },
                        )
                    })
                    .inner
                } else {
                    false
                };
                if changed {
                    self.run_update(id);
                }
            }
        }

        for (socket_canvas, highlight) in socket_highlights {
            let center = self.view.canvas_to_screen(origin, socket_canvas);
            let radius = (7.0 * self.view.zoom).clamp(5.0, 10.0);
            painter.circle_filled(
                center,
                radius,
                Color32::from_rgba_premultiplied(highlight.r(), highlight.g(), highlight.b(), 45),
            );
            painter.circle_stroke(center, radius, Stroke::new(1.5_f32, Color32::WHITE));
        }

        if let InteractionState::BoxSelecting {
            start_canvas,
            current_canvas,
        } = &self.interaction_state
        {
            draw_box_select(
                painter,
                self.view.canvas_to_screen(origin, *start_canvas),
                self.view.canvas_to_screen(origin, *current_canvas),
            );
        }

        if let InteractionState::CuttingWire { path } = &self.interaction_state {
            let screen_pts: Vec<egui::Pos2> = path
                .iter()
                .map(|&p| self.view.canvas_to_screen(origin, p))
                .collect();
            if screen_pts.len() >= 2 {
                draw_knife_line(painter, &screen_pts);
            }
        }

        if self.minimap_visible {
            let (info, minimap_rect) =
                minimap::compute_minimap(layout.node_rects.values().copied(), rect);
            minimap::draw_minimap(
                painter,
                &info,
                &self.graph,
                &layout.node_rects,
                &self.view,
                rect,
            );
            if !self.interaction_state.use_fast_rendering() {
                self.raise_minimap_hit_target(ui, minimap_rect);
            }
        }

        ui.set_clip_rect(previous_clip);
    }

    pub(crate) fn hovered_socket(&self, responses: &GraphResponses) -> Option<SocketId> {
        responses
            .sockets
            .iter()
            .filter(|(_, response)| response.hovered())
            .min_by_key(|(socket_id, _)| {
                (
                    socket_id.node.0,
                    match socket_id.direction {
                        SocketDirection::Input => 0,
                        SocketDirection::Output => 1,
                    },
                    socket_id.index,
                )
            })
            .map(|(&socket_id, _)| socket_id)
    }

    pub(crate) fn show_socket_tooltip(
        &self,
        responses: &GraphResponses,
        socket_id: Option<SocketId>,
    ) {
        let Some(socket_id) = socket_id else {
            return;
        };
        let Some(response) = responses.sockets.get(&socket_id) else {
            return;
        };
        response.clone().on_hover_ui(|ui| {
            self.socket_tooltip_ui(ui, socket_id);
        });
    }

    fn socket_tooltip_ui(&self, ui: &mut egui::Ui, socket_id: SocketId) {
        let Some((node_title, socket)) = self.socket_ref(socket_id) else {
            return;
        };
        ui.set_max_width(340.0);
        let socket_name = if socket.name.is_empty() {
            "(unnamed)"
        } else {
            &socket.name
        };
        ui.label(RichText::new(format!("{node_title}.{socket_name}")).strong());
        ui.separator();

        tooltip_row(
            ui,
            "Direction",
            match socket_id.direction {
                SocketDirection::Input => "Input",
                SocketDirection::Output => "Output",
            },
        );
        tooltip_row(ui, "Declared type", &socket.type_name);
        if socket.effective_type() != socket.type_name {
            tooltip_row(ui, "Current type", socket.effective_type());
        }
        if let Some(resolved) = &socket.resolved_type {
            tooltip_row(ui, "Resolved type", resolved);
        }
        tooltip_row(
            ui,
            "Supports",
            self.socket_supported_types(socket_id, socket),
        );

        let connections = self.connected_socket_labels(socket_id);
        tooltip_row(
            ui,
            "Connection",
            if connections.is_empty() {
                "Unconnected".to_owned()
            } else {
                connections.join(", ")
            },
        );
    }

    fn socket_supported_types(&self, socket_id: SocketId, socket: &Socket) -> String {
        match socket_id.direction {
            SocketDirection::Input => {
                if socket.type_name == "Any" {
                    return "Any".to_owned();
                }
                let mut supported = vec![socket.type_name.clone()];
                supported.extend(socket.allowed.iter().cloned());
                supported.sort();
                supported.dedup();
                supported.join(", ")
            }
            SocketDirection::Output => socket.type_name.clone(),
        }
    }

    fn connected_socket_labels(&self, socket_id: SocketId) -> Vec<String> {
        self.graph
            .connections
            .iter()
            .filter_map(|conn| {
                let other = if conn.from == socket_id {
                    conn.to
                } else if conn.to == socket_id {
                    conn.from
                } else {
                    return None;
                };
                self.socket_ref(other).map(|(node_title, socket)| {
                    let socket_name = if socket.name.is_empty() {
                        "(unnamed)"
                    } else {
                        &socket.name
                    };
                    format!("{node_title}.{socket_name}")
                })
            })
            .collect()
    }

    fn socket_ref(&self, socket_id: SocketId) -> Option<(&str, &Socket)> {
        let node = self.graph.nodes.get(&socket_id.node)?;
        let socket = match socket_id.direction {
            SocketDirection::Input => node.inputs.get(socket_id.index)?,
            SocketDirection::Output => node.outputs.get(socket_id.index)?,
        };
        Some((&node.title, socket))
    }

    fn socket_highlight_cluster(&self, socket_id: SocketId) -> Vec<SocketId> {
        let mut sockets = vec![socket_id];
        for conn in &self.graph.connections {
            if conn.from == socket_id {
                sockets.push(conn.to);
            } else if conn.to == socket_id {
                sockets.push(conn.from);
            }
        }
        sockets.sort_by_key(|socket_id| {
            (
                socket_id.node.0,
                match socket_id.direction {
                    SocketDirection::Input => 0,
                    SocketDirection::Output => 1,
                },
                socket_id.index,
            )
        });
        sockets.dedup();
        sockets
    }

    fn socket_display_color(&self, socket_id: SocketId) -> Color32 {
        self.socket_ref(socket_id)
            .map(|(_, socket)| self.registry.socket_display(socket).0)
            .unwrap_or(Color32::from_rgb(160, 160, 160))
    }

    pub(crate) fn show_frame_rename(&mut self, ctx: &egui::Context) {
        let Some(state) = &mut self.frame_rename else {
            return;
        };

        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Rename Frame")
            .id(egui::Id::new("node_graph_rename_frame"))
            .fixed_pos(state.screen_pos)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.text)
                        .desired_width(240.0)
                        .hint_text("Frame name"),
                );
                if response.lost_focus()
                    && self.input_bindings.consume_shortcut_ctx(
                        ui.ctx(),
                        &["node_graph.rename_edit"],
                        "confirm",
                    )
                {
                    apply = true;
                } else {
                    // Re-requesting focus here would mask the surrender above,
                    // since `lost_focus()` reads live state, not a frame snapshot.
                    response.request_focus();
                }
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if self
            .input_bindings
            .consume_shortcut_ctx(ctx, &["node_graph.rename_edit"], "cancel")
        {
            cancel = true;
        }
        if apply {
            if let Some(state) = self.frame_rename.take() {
                self.push_undo_snapshot();
                if let Some(frame) = self
                    .graph
                    .frames
                    .iter_mut()
                    .find(|frame| frame.id == state.frame_id)
                {
                    frame.label = state.text;
                }
            }
        } else if cancel {
            self.frame_rename = None;
        }
    }

    /// Inline rename overlay for a node (Phase 2, F2) — same mechanism as
    /// `show_frame_rename`, writing to `node.title` instead.
    pub(crate) fn show_node_rename(&mut self, ctx: &egui::Context) {
        let Some(state) = &mut self.node_rename else {
            return;
        };

        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Rename Node")
            .id(egui::Id::new("node_graph_rename_node"))
            .fixed_pos(state.screen_pos)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.text)
                        .desired_width(240.0)
                        .hint_text("Node name"),
                );
                if response.lost_focus()
                    && self.input_bindings.consume_shortcut_ctx(
                        ui.ctx(),
                        &["node_graph.rename_edit"],
                        "confirm",
                    )
                {
                    apply = true;
                } else {
                    // Re-requesting focus here would mask the surrender above,
                    // since `lost_focus()` reads live state, not a frame snapshot.
                    response.request_focus();
                }
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if self
            .input_bindings
            .consume_shortcut_ctx(ctx, &["node_graph.rename_edit"], "cancel")
        {
            cancel = true;
        }
        if apply {
            if let Some(state) = self.node_rename.take() {
                self.push_undo_snapshot();
                if let Some(node) = self.graph.nodes.get_mut(&state.node_id) {
                    node.title = state.text;
                }
                if let (Some(instance), Some(node)) = (
                    self.runtime.get_mut(&state.node_id),
                    self.graph.nodes.get(&state.node_id),
                ) {
                    instance.set_bound_title(&node.title);
                }
                self.run_update(state.node_id);
            }
        } else if cancel {
            self.node_rename = None;
        }
    }
}

fn tooltip_row(ui: &mut egui::Ui, label: &str, value: impl Into<String>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{label}:")).weak());
        ui.label(value.into());
    });
}

#[cfg(test)]
mod render_tests {
    use std::collections::HashSet;
    use std::rc::Rc;

    use egui::Pos2;

    use super::*;
    use crate::model::SocketDirection;
    use crate::runtime::NodeTypeRegistry;

    /// The four control points of the drag preview painted for a wire drag
    /// anchored at a socket of `direction`, in the order `draw_wire`
    /// emitted them.
    fn preview_curve(direction: SocketDirection, pointer: Pos2) -> [Pos2; 4] {
        let context = egui::Context::default();
        let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let node = widget
            .add_node_at("Reroute", Pos2::new(400.0, 300.0))
            .expect("built-in reroute node");
        let from = SocketId {
            node,
            index: 0,
            direction,
        };

        context.begin_pass(egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            context.clone(),
            egui::Id::new("wire-preview-shape-test"),
            egui::UiBuilder::new().max_rect(screen_rect),
        );
        let origin = ui.available_rect_before_wrap().min;
        let from_canvas = widget.build_layout(origin).socket_screen_pos[&from];
        widget.interaction_state = InteractionState::DraggingWire {
            from,
            from_canvas: widget.view.screen_to_canvas(origin, from_canvas),
            current_canvas: widget.view.screen_to_canvas(origin, pointer),
            restore_on_cancel: false,
            connectable: Rc::new(HashSet::new()),
        };
        widget.show(&mut ui);
        let mut output = context.end_pass();
        output.textures_delta.clear();

        let curves: Vec<[Pos2; 4]> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::CubicBezier(curve) => Some(curve.points),
                _ => None,
            })
            .collect();
        // The wire is painted twice, as its shadow and its colored stroke.
        assert_eq!(curves.len(), 2, "expected only the drag preview");
        curves[0]
    }

    #[test]
    fn a_preview_anchored_at_an_output_leaves_it_to_the_right() {
        let curve = preview_curve(SocketDirection::Output, Pos2::new(700.0, 500.0));

        assert!(curve[1].x > curve[0].x, "leaves the output rightwards");
        assert!(curve[2].x < curve[3].x, "enters the free end leftwards");
    }

    #[test]
    fn a_preview_anchored_at_an_input_enters_it_from_the_left() {
        // Pointer to the left of the anchor: drawn output-first the curve
        // would leave the input rightwards and double back, looping.
        let curve = preview_curve(SocketDirection::Input, Pos2::new(200.0, 500.0));

        assert!(curve[1].x > curve[0].x, "leaves the free end rightwards");
        assert!(curve[2].x < curve[3].x, "enters the input leftwards");
        assert!(
            curve[3].x > curve[0].x,
            "the anchored input is the curve's end, not its start"
        );
    }
}
