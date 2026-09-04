//! Modal graph pointer-transition coordination.
//!
//! This module owns the ordering and cancellation rules for pointer-driven
//! gesture transitions. The graph widget and input dispatcher consume its
//! inherent methods. It depends on transient interaction state, per-frame
//! responses, layout data, and the selection and wire owners. It does not own
//! global shortcut/menu dispatch, response allocation, selection policy, or
//! wire topology algorithms.

use std::rc::Rc;

use egui::{Pos2, Rect, Vec2};

use super::interaction_state::{
    DragAxis, DragConstraint, InteractionState, constrain_drag_position, rebase_drag_offset,
    toggle_drag_axis,
};
use super::layout::GraphWidgetLayout;
use super::response::GraphResponses;
use super::widget::NodeGraphWidget;
use crate::model::{FrameId, NodeId, SocketDirection};
use crate::support::egui_position;

impl NodeGraphWidget {
    /// Handles device drivers that encode a cancel gesture as another
    /// Primary press while a primary-started modal drag is already active.
    /// The duplicate event is removed before any widget sees it and the drag
    /// is cancelled semantically, so no synthetic pointer event can leak to
    /// panel or window handling.
    ///
    /// # Parameters
    /// - `raw_input`: Input consumed by this operation.
    pub fn filter_modal_raw_input(&mut self, raw_input: &mut egui::RawInput) -> bool {
        if !matches!(
            self.interaction_state,
            InteractionState::DraggingNode { .. } | InteractionState::DraggingWire { .. }
        ) {
            return false;
        }
        let previous_len = raw_input.events.len();
        raw_input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                }
            )
        });
        if raw_input.events.len() == previous_len {
            return false;
        }
        self.cancel_modal_drag()
    }

    /// Cancels the current modal node/link drag without synthesizing pointer
    /// input. Raw-input adapters use this after consuming device-specific
    /// duplicate events.
    pub fn cancel_modal_drag(&mut self) -> bool {
        let restore_snapshot = match self.interaction_state {
            InteractionState::DraggingNode { .. } => true,
            InteractionState::DraggingWire {
                restore_on_cancel, ..
            } => restore_on_cancel,
            _ => return false,
        };
        if restore_snapshot {
            self.cancel_undo_snapshot();
        }
        self.interaction_state = InteractionState::Idle;
        true
    }

    /// Cancels an active node or wire drag before ordinary graph input can
    /// claim the secondary button. Older binding files may not contain the
    /// modal action yet, so secondary remains the guaranteed fallback.
    fn cancel_active_drag(&mut self, ui: &egui::Ui) -> bool {
        let (context, cancel_action) = match self.interaction_state {
            InteractionState::DraggingNode { .. } => ("node_graph.drag_node", "cancel_move"),
            InteractionState::DraggingWire { .. } => ("node_graph.drag_wire", "cancel_link"),
            _ => return false,
        };
        let modifiers = ui.input(|input| input.modifiers);
        let configured_button = self
            .input_bindings
            .pointer_trigger(&[context], cancel_action, modifiers)
            .map(|(button, _)| button);
        let cancel = ui.input(|input| {
            let active = |button| {
                input.pointer.button_pressed(button)
                    || input.pointer.button_down(button)
                    || input.pointer.button_released(button)
            };
            active(egui::PointerButton::Secondary) || configured_button.is_some_and(active)
        });
        if cancel {
            self.cancel_modal_drag();
        }
        cancel
    }

    #[allow(clippy::too_many_arguments)]
    fn idle_transition(
        &mut self,
        ui: &egui::Ui,
        responses: &GraphResponses,
        pointer_canvas: Option<Pos2>,
        origin: Pos2,
        layout: &GraphWidgetLayout,
    ) -> InteractionState {
        let Some(pc) = pointer_canvas else {
            return InteractionState::Idle;
        };
        if !self.editing_enabled {
            return self.read_only_idle_transition(ui, responses, pc, origin, layout);
        }
        let primary_button = self
            .input_bindings
            .pointer_button(&["node_graph"], "select_move")
            .unwrap_or(egui::PointerButton::Primary);
        let connect_button = self
            .input_bindings
            .pointer_button(&["node_graph.socket", "node_graph"], "connect")
            .unwrap_or(primary_button);

        let current_screen_pos = self.view.canvas_to_screen(origin, pc);
        let press_screen_pos = ui
            .input(|i| i.pointer.press_origin())
            .unwrap_or(current_screen_pos);

        let options_button = self
            .input_bindings
            .pointer_button(&["node_graph"], "options")
            .unwrap_or(egui::PointerButton::Secondary);
        if ui.input(|i| i.pointer.button_down(options_button)) {
            return InteractionState::Idle;
        }
        let modifiers = ui.input(|i| i.modifiers);
        let ctrl = modifiers.ctrl;
        // A drag from a connected output adds a second link; the move
        // modifier picks up an existing one instead. An input holds a single
        // link, so dragging one always moves it and needs no modifier.
        let move_link = self
            .input_bindings
            .pointer_trigger(&["node_graph.socket", "node_graph"], "move_link", modifiers)
            .is_some();

        for (&sid, response) in &responses.sockets {
            if !response.drag_started_by(connect_button) {
                continue;
            }
            let Some(&spos) = layout.socket_screen_pos.get(&sid) else {
                continue;
            };
            if move_link
                && let Some(anchor) = self.movable_output_link(sid, current_screen_pos, layout)
                && let Some(&anchor_spos) = layout.socket_screen_pos.get(&anchor)
            {
                // The input the link keeps hanging from becomes the anchor,
                // exactly as the source output does when a connected input
                // is dragged; the free end then looks for another output.
                return self.start_link_move(anchor, anchor_spos, origin, pc);
            }
            if sid.direction == SocketDirection::Input
                && let Some(src) = self
                    .graph
                    .connections
                    .iter()
                    .find(|c| c.to == sid)
                    .map(|c| c.from)
                && let Some(&src_spos) = layout.socket_screen_pos.get(&src)
            {
                self.push_undo_snapshot();
                self.graph.disconnect_input(sid);
                self.run_update(sid.node);
                return InteractionState::DraggingWire {
                    from: src,
                    from_canvas: self.view.screen_to_canvas(origin, src_spos),
                    current_canvas: pc,
                    restore_on_cancel: true,
                    connectable: Rc::new(self.connectable_nodes(src)),
                };
            }
            return InteractionState::DraggingWire {
                from: sid,
                from_canvas: self.view.screen_to_canvas(origin, spos),
                current_canvas: pc,
                restore_on_cancel: false,
                connectable: Rc::new(self.connectable_nodes(sid)),
            };
        }

        for (&id, response) in &responses.collapse_toggles {
            if response.clicked_by(primary_button) {
                self.push_undo_snapshot();
                self.toggle_collapsed_for_node(id);
                return InteractionState::Idle;
            }
        }

        for (&id, responses) in &responses.nodes {
            if responses.body.clicked_by(primary_button)
                || responses.header.clicked_by(primary_button)
            {
                self.select_node(id, ctrl);
                return InteractionState::Idle;
            }
            let drag_started = responses.header.drag_started_by(primary_button)
                || responses.body.drag_started_by(primary_button);
            // The socket halves of a reroute point leave only a sliver of
            // body between them, so the modifier picks its link up from the
            // whole point; without it the point is moved as any other node.
            if drag_started
                && move_link
                && let Some(anchor) = self.movable_reroute_link(id, current_screen_pos, layout)
                && let Some(&anchor_spos) = layout.socket_screen_pos.get(&anchor)
            {
                return self.start_link_move(anchor, anchor_spos, origin, pc);
            }
            if drag_started && let Some(node) = self.graph.nodes.get(&id) {
                let start_pos = node.pos;
                let node_pos = egui_position(start_pos).to_vec2();
                if !node.selected || ctrl {
                    self.select_node(id, ctrl);
                }
                self.push_undo_snapshot();
                return InteractionState::DraggingNode {
                    node_id: id,
                    offset: pc.to_vec2() - node_pos,
                    constraint: None,
                };
            }
        }

        if responses.frames.values().any(egui::Response::clicked)
            && self
                .node_at_screen_pos(responses, current_screen_pos)
                .is_none()
            && let Some(id) = self.frame_at_screen_pos(responses, layout, current_screen_pos)
        {
            self.select_frame(id, ctrl);
            return InteractionState::Idle;
        }

        if responses.frames.values().any(egui::Response::drag_started) {
            if self
                .node_at_screen_pos(responses, press_screen_pos)
                .is_some()
            {
                return InteractionState::Idle;
            }
            if let Some(id) = self.frame_at_screen_pos(responses, layout, press_screen_pos) {
                self.select_frame(id, ctrl);
                self.push_undo_snapshot();
                return InteractionState::DraggingFrame {
                    frame_id: id,
                    last_canvas: pc,
                };
            }
        }

        // Checked before the plain-click deselect below: egui fires both
        // `clicked()` and `double_clicked()` on a double-click's second
        // press, and inserting a reroute shouldn't also clear the selection
        // as a side effect (Phase 6.2).
        let reroute_button = self
            .input_bindings
            .pointer_button(&["node_graph.canvas", "node_graph"], "insert_reroute")
            .unwrap_or(primary_button);
        // Two ways onto a wire: the plain double-click, and the single
        // click of whichever modifier-qualified binding matches what is
        // held right now (Command-click as shipped).
        let modified_click = self
            .input_bindings
            .pointer_trigger(
                &["node_graph.canvas", "node_graph"],
                "insert_reroute",
                modifiers,
            )
            .is_some_and(|(button, gesture)| {
                gesture == input_bindings::PointerGesture::Click
                    && responses.canvas.clicked_by(button)
            });
        if (responses.canvas.double_clicked_by(reroute_button) || modified_click)
            && let Some(idx) = self.wire_near_point(pc, layout)
        {
            self.insert_reroute_on_wire(idx, pc);
            return InteractionState::Idle;
        }

        if responses.canvas.clicked_by(primary_button) && !ctrl {
            for node in self.graph.nodes.values_mut() {
                node.selected = false;
            }
            for frame in &mut self.graph.frames {
                frame.selected = false;
            }
        }

        if responses.canvas.drag_started_by(primary_button) {
            return InteractionState::BoxSelecting {
                start_canvas: pc,
                current_canvas: pc,
            };
        }

        InteractionState::Idle
    }

    fn read_only_idle_transition(
        &mut self,
        ui: &egui::Ui,
        responses: &GraphResponses,
        pointer_canvas: Pos2,
        origin: Pos2,
        layout: &GraphWidgetLayout,
    ) -> InteractionState {
        let primary_button = self
            .input_bindings
            .pointer_button(&["node_graph"], "select_move")
            .unwrap_or(egui::PointerButton::Primary);
        let ctrl = ui.input(|input| input.modifiers.ctrl);
        for (&id, response) in &responses.nodes {
            if response.body.clicked_by(primary_button)
                || response.header.clicked_by(primary_button)
            {
                self.select_node(id, ctrl);
                return InteractionState::Idle;
            }
        }

        let screen_pos = self.view.canvas_to_screen(origin, pointer_canvas);
        if responses.frames.values().any(egui::Response::clicked)
            && self.node_at_screen_pos(responses, screen_pos).is_none()
            && let Some(id) = self.frame_at_screen_pos(responses, layout, screen_pos)
        {
            self.select_frame(id, ctrl);
            return InteractionState::Idle;
        }

        if responses.canvas.clicked_by(primary_button) && !ctrl {
            for node in self.graph.nodes.values_mut() {
                node.selected = false;
            }
            for frame in &mut self.graph.frames {
                frame.selected = false;
            }
        }
        if responses.canvas.drag_started_by(primary_button) {
            return InteractionState::BoxSelecting {
                start_canvas: pointer_canvas,
                current_canvas: pointer_canvas,
            };
        }
        InteractionState::Idle
    }

    fn update_panning(
        &mut self,
        response: &egui::Response,
        pointer: Option<Pos2>,
        last_screen: Pos2,
    ) -> InteractionState {
        if response.dragged()
            && let Some(pp) = pointer
        {
            self.view.pan += pp - last_screen;
            return InteractionState::Panning { last_screen: pp };
        }
        InteractionState::Idle
    }

    fn update_drag_node(
        &mut self,
        ui: &mut egui::Ui,
        pointer_canvas: Option<Pos2>,
        node_id: NodeId,
        mut offset: Vec2,
        mut constraint: Option<DragConstraint>,
        layout: &GraphWidgetLayout,
    ) -> InteractionState {
        let modifiers = ui.input(|input| input.modifiers);
        let requested_axis = if self.input_bindings.consume_shortcut_once(
            ui,
            &["node_graph.drag_node"],
            "constrain_x",
        ) {
            Some(DragAxis::X)
        } else if self.input_bindings.consume_shortcut_once(
            ui,
            &["node_graph.drag_node"],
            "constrain_y",
        ) {
            Some(DragAxis::Y)
        } else {
            None
        };
        if let Some(requested_axis) = requested_axis {
            let position = self
                .graph
                .nodes
                .get(&node_id)
                .map_or(Pos2::ZERO, |node| egui_position(node.pos));
            let next_constraint = toggle_drag_axis(constraint, requested_axis, position);
            if constraint.is_some()
                && next_constraint.is_none()
                && let Some(pointer) = pointer_canvas
            {
                // Free movement resumes from the node's current position,
                // rather than jumping to the unconstrained pointer offset.
                offset = rebase_drag_offset(pointer, position);
            }
            constraint = next_constraint;
        }

        if let Some(pc) = pointer_canvas {
            // The active-drag binding controls Blender-style grid snap.
            // Only the dragged node itself snaps to the grid; every
            // other selected node moves by the same resulting delta,
            // keeping the whole selection's relative layout intact.
            let snap = self
                .input_bindings
                .pointer_trigger(&["node_graph.drag_node"], "snap_to_grid", modifiers)
                .is_some();
            let new_pos =
                constrain_drag_position((pc.to_vec2() - offset).to_pos2(), constraint, snap);
            let delta = self
                .graph
                .nodes
                .get(&node_id)
                .map(|n| new_pos - egui_position(n.pos))
                .unwrap_or(Vec2::ZERO);
            for n in self.graph.nodes.values_mut() {
                if n.selected {
                    n.pos.translate(delta.x, delta.y);
                }
            }
        }

        let confirm = self
            .input_bindings
            .pointer_trigger(&["node_graph.drag_node"], "confirm_move", modifiers)
            .is_some_and(|(button, _)| ui.input(|input| input.pointer.button_released(button)));
        if !confirm {
            return InteractionState::DraggingNode {
                node_id,
                offset,
                constraint,
            };
        }

        let drop_layout = self.build_layout_excluding(Pos2::ZERO, Some(node_id));
        self.try_wire_insert(node_id, pointer_canvas, &drop_layout);
        let selected: Vec<NodeId> = self
            .graph
            .nodes
            .values()
            .filter(|node| node.selected)
            .map(|node| node.id)
            .collect();
        self.resolve_frame_membership_on_drop(&selected, layout);
        InteractionState::Idle
    }

    fn update_drag_frame(
        &mut self,
        ui: &egui::Ui,
        pointer_canvas: Option<Pos2>,
        frame_id: FrameId,
        last_canvas: Pos2,
    ) -> InteractionState {
        let button = self
            .input_bindings
            .pointer_button(&["node_graph"], "select_move")
            .unwrap_or(egui::PointerButton::Primary);
        if ui.input(|i| i.pointer.button_down(button)) {
            if let Some(pc) = pointer_canvas {
                let delta = pc - last_canvas;
                self.move_selected_frame_nodes(frame_id, delta);
                return InteractionState::DraggingFrame {
                    frame_id,
                    last_canvas: pc,
                };
            }
            return InteractionState::DraggingFrame {
                frame_id,
                last_canvas,
            };
        }
        InteractionState::Idle
    }

    /// Drives `InteractionState::PlacingNodes` (Phase 1.2): moves every
    /// selected node by the pointer's per-frame delta until the primary
    /// button confirms (with a wire-splice check when exactly one node is
    /// being placed, matching `update_drag_node`'s drop behavior) or
    /// Escape/secondary-click cancels by undoing the add/duplicate/paste.
    /// The confirm/cancel checks are skipped entirely on `just_entered`'s
    /// frame — see the field's doc comment on `InteractionState::PlacingNodes`.
    fn update_placing_nodes(
        &mut self,
        ui: &egui::Ui,
        pointer_canvas: Option<Pos2>,
        anchor_canvas: Pos2,
        just_entered: bool,
        layout: &GraphWidgetLayout,
    ) -> InteractionState {
        if !just_entered {
            let modifiers = ui.input(|input| input.modifiers);
            let cancel = self
                .input_bindings
                .pointer_trigger(&["node_graph.placement"], "cancel", modifiers)
                .is_some_and(|(button, _)| ui.input(|input| input.pointer.button_pressed(button)));
            if cancel {
                self.undo();
                return InteractionState::Idle;
            }
            let confirm = self
                .input_bindings
                .pointer_trigger(&["node_graph.placement"], "confirm", modifiers)
                .is_some_and(|(button, _)| ui.input(|input| input.pointer.button_pressed(button)));
            if confirm {
                let selected: Vec<NodeId> = self
                    .graph
                    .nodes
                    .values()
                    .filter(|node| node.selected)
                    .map(|node| node.id)
                    .collect();
                if let [only] = selected[..] {
                    let drop_layout = self.build_layout_excluding(Pos2::ZERO, Some(only));
                    self.try_wire_insert(only, pointer_canvas, &drop_layout);
                }
                self.resolve_frame_membership_on_drop(&selected, layout);
                return InteractionState::Idle;
            }
        }
        let Some(pc) = pointer_canvas else {
            return InteractionState::PlacingNodes {
                anchor_canvas,
                just_entered: false,
            };
        };
        let delta = pc - anchor_canvas;
        if delta != Vec2::ZERO {
            for n in self.graph.nodes.values_mut() {
                if n.selected {
                    n.pos.translate(delta.x, delta.y);
                }
            }
        }
        InteractionState::PlacingNodes {
            anchor_canvas: pc,
            just_entered: false,
        }
    }

    fn update_box_select(
        &mut self,
        ui: &egui::Ui,
        pointer_canvas: Option<Pos2>,
        layout: &GraphWidgetLayout,
        start_canvas: Pos2,
        mut current_canvas: Pos2,
    ) -> InteractionState {
        let button = self
            .input_bindings
            .pointer_button(&["node_graph"], "select_move")
            .unwrap_or(egui::PointerButton::Primary);
        if ui.input(|i| i.pointer.button_down(button)) {
            if let Some(pc) = pointer_canvas {
                current_canvas = pc;
            }
            return InteractionState::BoxSelecting {
                start_canvas,
                current_canvas,
            };
        }
        let select_rect = egui::Rect::from_two_pos(start_canvas, current_canvas);
        let shift = ui.input(|i| i.modifiers.shift);
        let ctrl = ui.input(|i| i.modifiers.ctrl);
        if !shift && !ctrl {
            for n in self.graph.nodes.values_mut() {
                n.selected = false;
            }
            for frame in &mut self.graph.frames {
                frame.selected = false;
            }
        }
        for (id, widget) in &layout.nodes {
            if select_rect.intersects(widget.node_rect())
                && let Some(n) = self.graph.nodes.get_mut(id)
            {
                n.selected = !ctrl;
            }
        }
        for (id, rect) in &layout.frame_rects {
            if select_rect.intersects(*rect)
                && let Some(frame) = self.graph.frames.iter_mut().find(|frame| frame.id == *id)
            {
                frame.selected = !ctrl;
            }
        }
        InteractionState::Idle
    }

    /// Pans the view while the pointer sits within `MARGIN` of (or past) the
    /// canvas edge during a drag (Phase 6.1). `DraggingNode`, `DraggingWire`,
    /// `BoxSelecting`, and `PlacingNodes` all derive their target position
    /// from `pointer_canvas` on the *next* frame, so nudging `view.pan` here
    /// is enough to move the drag correctly — no per-state position math
    /// needed.
    fn edge_auto_pan(&mut self, pointer: Pos2, canvas_rect: Rect) {
        const MARGIN: f32 = 24.0;
        const MAX_SPEED: f32 = 15.0;
        const GAIN: f32 = 0.15;

        let overshoot_left = (canvas_rect.min.x + MARGIN) - pointer.x;
        let overshoot_right = pointer.x - (canvas_rect.max.x - MARGIN);
        let overshoot_top = (canvas_rect.min.y + MARGIN) - pointer.y;
        let overshoot_bottom = pointer.y - (canvas_rect.max.y - MARGIN);

        let mut delta = Vec2::ZERO;
        if overshoot_left > 0.0 {
            delta.x += (overshoot_left * GAIN).min(MAX_SPEED);
        } else if overshoot_right > 0.0 {
            delta.x -= (overshoot_right * GAIN).min(MAX_SPEED);
        }
        if overshoot_top > 0.0 {
            delta.y += (overshoot_top * GAIN).min(MAX_SPEED);
        } else if overshoot_bottom > 0.0 {
            delta.y -= (overshoot_bottom * GAIN).min(MAX_SPEED);
        }

        if delta != Vec2::ZERO {
            self.view.pan += delta;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_interaction(
        &mut self,
        ui: &mut egui::Ui,
        responses: &GraphResponses,
        pointer: Option<Pos2>,
        pointer_canvas: Option<Pos2>,
        origin: Pos2,
        canvas_rect: Rect,
        layout: &GraphWidgetLayout,
    ) {
        // Keep modal cancellation in the interaction update. This is where
        // node dragging handled it before wire dragging gained the same
        // controls, and it ensures cancel wins over confirm for both modes.
        if self.cancel_active_drag(ui) {
            return;
        }
        let response = &responses.canvas;
        if self
            .input_bindings
            .consume_shortcut(ui, &["node_graph"], "cancel")
        {
            match self.interaction_state {
                // Cancelling placement reverts the add/duplicate/paste that
                // started it. Node and detached-wire drags restore their
                // pre-drag snapshots.
                InteractionState::PlacingNodes { .. } => self.undo(),
                InteractionState::DraggingNode { .. } => self.cancel_undo_snapshot(),
                InteractionState::DraggingWire {
                    restore_on_cancel: true,
                    ..
                } => self.cancel_undo_snapshot(),
                _ => {}
            }
            self.interaction_state = InteractionState::Idle;
            return;
        }

        let modifiers = ui.input(|input| input.modifiers);
        // `button_down` is global pointer state, not scoped to this widget —
        // without the hover/already-panning check, a middle-drag started
        // over a sibling widget (e.g. the logic analyzer above the graph)
        // would also pan the graph. Once a pan has started, keep following
        // the drag even if the pointer leaves the canvas rect.
        let pan_trigger = self
            .input_bindings
            .pointer_trigger(&["node_graph"], "pan", modifiers);
        let zoom_trigger =
            self.input_bindings
                .pointer_trigger(&["node_graph"], "zoom_drag", modifiers);
        let active_view_trigger = zoom_trigger.or(pan_trigger);
        let can_pan = matches!(
            self.interaction_state,
            InteractionState::Idle | InteractionState::Panning { .. }
        );
        let middle_down = can_pan
            && active_view_trigger
                .is_some_and(|(button, _)| ui.input(|input| input.pointer.button_down(button)))
            && (pointer.is_some() && response.hovered()
                || matches!(self.interaction_state, InteractionState::Panning { .. }));
        let cut_trigger =
            self.input_bindings
                .pointer_trigger(&["node_graph"], "cut_wires", modifiers);
        let can_cut = matches!(
            self.interaction_state,
            InteractionState::Idle | InteractionState::CuttingWire { .. }
        );
        let cutting = matches!(self.interaction_state, InteractionState::CuttingWire { .. });
        let right_down = self.editing_enabled
            && can_cut
            && cut_trigger.is_some_and(|(button, _)| {
                ui.input(|input| {
                    input.pointer.button_down(button)
                        && (cutting || input.pointer.button_pressed(button))
                })
            });

        if middle_down {
            if let Some(pp) = pointer {
                let delta =
                    if let InteractionState::Panning { last_screen } = self.interaction_state {
                        pp - last_screen
                    } else {
                        Vec2::ZERO
                    };
                if zoom_trigger.is_some() {
                    let factor = (1.0_f32 - delta.y * 0.005).clamp(0.5, 2.0);
                    if delta.y.abs() > 0.1 {
                        self.view.zoom_around(pp, origin, factor);
                    }
                } else {
                    self.view.pan += delta;
                }
                self.interaction_state = InteractionState::Panning { last_screen: pp };
            }
            return;
        }
        if matches!(self.interaction_state, InteractionState::Panning { .. }) {
            self.interaction_state = InteractionState::Idle;
        }

        if right_down {
            if let Some(pc) = pointer_canvas {
                match &mut self.interaction_state {
                    InteractionState::CuttingWire { path } => {
                        let min_step = 4.0 / self.view.zoom;
                        if path.last().is_none_or(|&last| last.distance(pc) > min_step) {
                            path.push(pc);
                        }
                    }
                    _ => self.interaction_state = InteractionState::CuttingWire { path: vec![pc] },
                }
            }
            return;
        }
        if matches!(self.interaction_state, InteractionState::CuttingWire { .. }) {
            let state = std::mem::replace(&mut self.interaction_state, InteractionState::Idle);
            if let InteractionState::CuttingWire { path } = state {
                self.apply_knife_cut(&path, layout);
            }
        }

        if matches!(self.interaction_state, InteractionState::Idle)
            && let Some(minimap) = &responses.minimap
            && let Some(pp) = minimap.response.hover_pos()
            && (minimap.response.drag_started() || minimap.response.dragged())
        {
            let canvas_pos = minimap.info.mini_to_canvas(pp);
            self.view.pan = (canvas_rect.center() - origin) - canvas_pos.to_vec2() * self.view.zoom;
            return;
        }

        if let Some(pp) = pointer
            && matches!(
                self.interaction_state,
                InteractionState::DraggingNode { .. }
                    | InteractionState::DraggingWire { .. }
                    | InteractionState::BoxSelecting { .. }
                    | InteractionState::PlacingNodes { .. }
            )
        {
            self.edge_auto_pan(pp, canvas_rect);
        }

        let state = std::mem::replace(&mut self.interaction_state, InteractionState::Idle);
        self.interaction_state = match state {
            InteractionState::Idle => {
                self.idle_transition(ui, responses, pointer_canvas, origin, layout)
            }
            InteractionState::Panning { last_screen } => {
                self.update_panning(response, pointer, last_screen)
            }
            InteractionState::DraggingNode {
                node_id,
                offset,
                constraint,
            } => self.update_drag_node(ui, pointer_canvas, node_id, offset, constraint, layout),
            InteractionState::DraggingFrame {
                frame_id,
                last_canvas,
            } => self.update_drag_frame(ui, pointer_canvas, frame_id, last_canvas),
            InteractionState::DraggingWire {
                from,
                from_canvas,
                current_canvas,
                restore_on_cancel,
                connectable,
            } => self.update_drag_wire(
                ui,
                pointer,
                pointer_canvas,
                responses,
                layout,
                from,
                from_canvas,
                current_canvas,
                restore_on_cancel,
                connectable,
            ),
            InteractionState::BoxSelecting {
                start_canvas,
                current_canvas,
            } => self.update_box_select(ui, pointer_canvas, layout, start_canvas, current_canvas),
            InteractionState::CuttingWire { path } => {
                self.update_cut_wire(ui, pointer_canvas, layout, path)
            }
            InteractionState::PlacingNodes {
                anchor_canvas,
                just_entered,
            } => self.update_placing_nodes(ui, pointer_canvas, anchor_canvas, just_entered, layout),
        };
    }
}

#[cfg(test)]
mod interaction_tests;
