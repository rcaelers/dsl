//! Global graph input and menu dispatch.
//!
//! This module owns viewport shortcuts, action dispatch, and context-menu
//! targeting for each input pass. It delegates modal pointer transitions and
//! graph mutations to their owning modules.

use egui::text_selection::LabelSelectionState;
use egui::{Pos2, Rect};

use super::action::ActionEffect;
use super::interaction_state::InteractionState;
use super::layout::GraphWidgetLayout;
use super::menu::{ContextMenuState, build_context_entries};
use super::response::{ContextClickTarget, GraphResponses};
use super::widget::NodeGraphWidget;
use crate::model::NodeId;
use crate::widget::menu::dispatch_menu_shortcut;

impl NodeGraphWidget {
    fn apply_effect(&mut self, effect: ActionEffect, pointer_canvas: Option<Pos2>) {
        match effect {
            ActionEffect::None => {}
            ActionEffect::ResetInteraction => self.interaction_state = InteractionState::Idle,
            ActionEffect::EnterPlacement => {
                // Without a live pointer there is nothing to follow; the
                // action already fell back to fixed-position placement.
                if let Some(anchor_canvas) = pointer_canvas {
                    self.interaction_state = InteractionState::PlacingNodes {
                        anchor_canvas,
                        just_entered: true,
                    };
                }
            }
        }
    }

    fn node_has_hidden_sockets(&self, node_id: NodeId) -> bool {
        self.graph.nodes.get(&node_id).is_some_and(|n| {
            n.inputs.iter().any(|s| s.hidden) || n.outputs.iter().any(|s| s.hidden)
        })
    }

    fn menu_collapsed_state(&self, context_node: Option<NodeId>) -> bool {
        if let Some(node_id) = context_node {
            return self
                .graph
                .nodes
                .get(&node_id)
                .is_some_and(|node| node.collapsed);
        }
        self.graph
            .nodes
            .values()
            .any(|node| node.selected && node.collapsed)
    }

    fn menu_muted_state(&self, context_node: Option<NodeId>) -> bool {
        if let Some(node_id) = context_node {
            return self
                .graph
                .nodes
                .get(&node_id)
                .is_some_and(|node| node.muted);
        }
        self.graph
            .nodes
            .values()
            .any(|node| node.selected && node.muted)
    }

    pub(crate) fn handle_input(
        &mut self,
        ui: &mut egui::Ui,
        responses: &GraphResponses,
        pointer: Option<Pos2>,
        origin: Pos2,
        layout: &GraphWidgetLayout,
        canvas_rect: Rect,
    ) {
        let response = &responses.canvas;
        let (scroll, zoom_delta, zoom_modifier) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers.ctrl || i.modifiers.command || i.modifiers.mac_cmd,
            )
        });
        let has_scroll = scroll.length_sq() > 0.01;
        let has_zoom = zoom_modifier && (zoom_delta - 1.0).abs() > 0.001;
        if (has_scroll || has_zoom)
            && !self.menu.blocks_canvas_scroll(ui)
            && let Some(cursor) = pointer
            && canvas_rect.contains(cursor)
        {
            if has_zoom {
                self.view
                    .zoom_around(cursor, origin, zoom_delta.clamp(0.5, 2.0));
            } else if zoom_modifier && scroll.y.abs() > 0.1 {
                self.view
                    .zoom_around(cursor, origin, (1.0_f32 + scroll.y * 0.003).clamp(0.5, 2.0));
            } else if !zoom_modifier {
                self.view.pan += scroll;
            }
        }

        let pointer_canvas = pointer.map(|p| self.view.screen_to_canvas(origin, p));
        let fallback_paste_pos = pointer_canvas
            .or_else(|| pointer.map(|p| self.view.screen_to_canvas(origin, p)))
            .unwrap_or_else(|| self.view.screen_to_canvas(origin, canvas_rect.center()));
        let no_focus = ui.ctx().memory(|m| m.focused().is_none());
        // Label selections deliberately own the standard clipboard shortcuts:
        // `Event::Copy` otherwise reaches the graph's menu shortcut and replaces
        // a selected diagnostic or log entry with a node-graph payload.
        let label_text_selected = ui
            .ctx()
            .plugin::<LabelSelectionState>()
            .lock()
            .has_selection();

        if no_focus
            && pointer.is_some()
            && self
                .input_bindings
                .consume_shortcut(ui, &["node_graph"], "fit")
        {
            self.fit_graph_to_viewport(layout, canvas_rect, origin);
            return;
        }

        // Zoom-to-selection (Blender's numpad-`.`) and rename-active (F2)
        // are special-cased here, like Home above, rather than routed
        // through `self.hotkeys`: both need `layout`/`origin` (for viewport
        // fitting and for placing the rename popup at the node's screen
        // position) that the generic action dispatch doesn't carry.
        if no_focus
            && pointer.is_some()
            && self
                .input_bindings
                .consume_shortcut(ui, &["node_graph"], "fit_selection")
        {
            self.fit_selection_to_viewport(layout, canvas_rect, origin);
            return;
        }

        if no_focus
            && self.editing_enabled
            && self
                .input_bindings
                .consume_shortcut(ui, &["node_graph"], "rename")
            && let Some(active) = self.active_node
            && let Some(&header_rect) = layout.header_screen_rects.get(&active)
        {
            self.start_renaming_node(active, header_rect.left_bottom());
            return;
        }

        if no_focus && !label_text_selected {
            let any_selected = self.graph.nodes.values().any(|node| node.selected)
                || self.graph.frames.iter().any(|frame| frame.selected);
            let shortcut_entries = build_context_entries(ContextMenuState {
                registry: &self.registry,
                canvas_pos: fallback_paste_pos,
                screen_pos: pointer.unwrap_or(canvas_rect.center()),
                context_node: None,
                context_frame: None,
                any_frame_selected: self.graph.frames.iter().any(|frame| frame.selected),
                node_hidden: false,
                node_collapsed: self.menu_collapsed_state(None),
                node_muted: self.menu_muted_state(None),
                node_has_derived_cache: false,
                node_actions: &[],
                any_selected,
                can_paste: self.can_paste_nodes(),
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
                editing_enabled: self.editing_enabled,
                input_bindings: &self.input_bindings,
            });
            if let Some(action) = dispatch_menu_shortcut(ui, &shortcut_entries) {
                let effect = self.execute_action(action, ui.ctx(), pointer_canvas);
                self.apply_effect(effect, pointer_canvas);
            }
        }

        // Shift+A opens the Add search at the pointer (Blender's Add menu);
        // plain A/Alt+A (select-all/deselect-all) go through `self.hotkeys`
        // below as ordinary `GraphAction`s. This one stays special-cased
        // because positioning the popup needs the screen pointer/canvas
        // origin this registry's dispatch doesn't carry.
        let placing = matches!(
            self.interaction_state,
            InteractionState::PlacingNodes { .. }
        );
        if no_focus
            && self.editing_enabled
            && !placing
            && self
                .input_bindings
                .consume_shortcut(ui, &["node_graph"], "add")
        {
            let screen_pos = pointer.unwrap_or(canvas_rect.center());
            let canvas_pos = self.view.screen_to_canvas(origin, screen_pos);
            self.menu
                .open_add_popup(screen_pos, &self.registry, canvas_pos);
        }

        for action in self.hotkeys.dispatch(ui, &self.input_bindings) {
            let effect = self.execute_action(action, ui.ctx(), pointer_canvas);
            self.apply_effect(effect, pointer_canvas);
        }

        let cutting = matches!(self.interaction_state, InteractionState::CuttingWire { .. });
        let dragging_modal = matches!(
            self.interaction_state,
            InteractionState::DraggingNode { .. } | InteractionState::DraggingWire { .. }
        );

        // A modal drag owns secondary-click. Do not remember that press for
        // the ordinary graph context menu; otherwise releasing the cancel
        // button on the following frame can immediately open that menu.
        if !dragging_modal
            && let Some(context_screen_pos) = self.menu.context_trigger_pos(
                ui,
                pointer,
                !cutting && !placing,
                &self.input_bindings,
            )
            && let Some(context_target) =
                self.context_click_target_at(responses, layout, context_screen_pos)
        {
            let mut context_frame = None;
            let context_node = match context_target {
                ContextClickTarget::Canvas => None,
                ContextClickTarget::Node(id) => Some(id),
                ContextClickTarget::Frame(id) => {
                    if !self
                        .graph
                        .frames
                        .iter()
                        .any(|frame| frame.id == id && frame.selected)
                    {
                        self.select_frame(id, false);
                    }
                    context_frame = Some(id);
                    None
                }
            };
            let canvas_pos = self.view.screen_to_canvas(origin, context_screen_pos);
            let node_hidden = context_node.is_some_and(|id| self.node_has_hidden_sockets(id));
            let node_collapsed = self.menu_collapsed_state(context_node);
            let node_muted = self.menu_muted_state(context_node);
            let node_has_derived_cache =
                context_node.is_some_and(|id| self.derived_cache_nodes.contains(&id));
            let node_actions = context_node
                .and_then(|id| self.node_context_actions.get(&id))
                .cloned()
                .unwrap_or_default();
            let any_selected = self.graph.nodes.values().any(|n| n.selected)
                || self.graph.frames.iter().any(|frame| frame.selected);
            let can_paste = self.can_paste_nodes();
            let entries = build_context_entries(ContextMenuState {
                registry: &self.registry,
                canvas_pos,
                screen_pos: context_screen_pos,
                context_node,
                context_frame,
                any_frame_selected: self.graph.frames.iter().any(|frame| frame.selected),
                node_hidden,
                node_collapsed,
                node_muted,
                node_has_derived_cache,
                node_actions: &node_actions,
                any_selected,
                can_paste,
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
                editing_enabled: self.editing_enabled,
                input_bindings: &self.input_bindings,
            });
            self.menu.open_popup(context_screen_pos, entries);
        }

        // Shift+A opens the Add search at the pointer (Blender's Add menu);
        // plain A/Alt+A (select-all/deselect-all) are ordinary `GraphAction`s
        // dispatched through `self.hotkeys` below — this one stays
        // special-cased because positioning the popup needs the screen
        // pointer and canvas origin, which the generic action dispatch
        // doesn't carry.
        if no_focus
            && self.editing_enabled
            && !placing
            && self
                .input_bindings
                .consume_shortcut(ui, &["node_graph"], "add")
        {
            let screen_pos = pointer.unwrap_or(canvas_rect.center());
            let canvas_pos = self.view.screen_to_canvas(origin, screen_pos);
            self.menu
                .open_add_popup(screen_pos, &self.registry, canvas_pos);
        }

        let menu_owned_pointer = self.menu.is_open();
        if let Some(action) = self.menu.update(ui, response, pointer, !cutting) {
            let effect = self.execute_action(action, ui.ctx(), pointer_canvas);
            self.apply_effect(effect, pointer_canvas);
        }

        // A menu that was open at the start of this update owns the complete
        // pointer gesture, including an outside click used only to dismiss it.
        // The canvas responses were collected earlier in the frame, so without
        // this guard the same release would also clear the graph selection.
        if menu_owned_pointer {
            return;
        }

        self.update_interaction(
            ui,
            responses,
            pointer,
            pointer_canvas,
            origin,
            canvas_rect,
            layout,
        );
        if self.interaction_state.is_active() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
