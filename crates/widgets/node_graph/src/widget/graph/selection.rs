//! Node selection and frame-membership policy.
//!
//! This module owns selection transitions, coordinated frame movement, and
//! join-only frame membership on drop. It does not inspect raw input, render
//! responses, or mutate wire topology.

use egui::Vec2;

use super::layout::GraphWidgetLayout;
use super::widget::NodeGraphWidget;
use crate::model::{FrameId, NodeId};

impl NodeGraphWidget {
    pub(crate) fn select_node(&mut self, id: NodeId, toggle: bool) {
        if !toggle {
            for n in self.graph.nodes.values_mut() {
                n.selected = false;
            }
            for frame in &mut self.graph.frames {
                frame.selected = false;
            }
        }
        if let Some(node) = self.graph.nodes.get_mut(&id) {
            if toggle {
                node.selected = !node.selected;
            } else {
                node.selected = true;
            }
            // Blender-style "active" node: the properties panel follows the
            // most recent selection.
            if node.selected {
                self.set_active_node(id);
            }
        }
    }

    pub(crate) fn select_frame(&mut self, id: FrameId, toggle: bool) {
        if !toggle {
            for node in self.graph.nodes.values_mut() {
                node.selected = false;
            }
            for frame in &mut self.graph.frames {
                frame.selected = false;
            }
        }
        if let Some(frame) = self.graph.frames.iter_mut().find(|frame| frame.id == id) {
            if toggle {
                frame.selected = !frame.selected;
            } else {
                frame.selected = true;
            }
        }
    }

    pub(crate) fn move_selected_frame_nodes(&mut self, fallback_frame: FrameId, delta: Vec2) {
        let selected_frames: Vec<_> = self
            .graph
            .frames
            .iter()
            .filter(|frame| frame.selected)
            .map(|frame| frame.id)
            .collect();
        let target_frames = if selected_frames.is_empty() {
            vec![fallback_frame]
        } else {
            selected_frames
        };
        let mut moved = std::collections::HashSet::new();
        for frame_id in target_frames {
            let Some(frame) = self.graph.frames.iter().find(|frame| frame.id == frame_id) else {
                continue;
            };
            for &node_id in &frame.node_ids {
                if moved.insert(node_id)
                    && let Some(node) = self.graph.nodes.get_mut(&node_id)
                {
                    node.pos.translate(delta.x, delta.y);
                }
            }
        }
    }

    /// Frame that would join `node_id` if it were dropped right now — `None`
    /// if it's already a member of a frame (Phase 1.3): dragging can only
    /// ever *add* a node to a frame, never remove it. Membership only ever
    /// changes the other direction via the explicit "Remove from Frame"
    /// action, so a node that's already in a frame is never a candidate
    /// here — there is nothing to leave-and-rejoin, and no frame ever steals
    /// a node away from another one by drag alone. Run live (not against a
    /// gesture-start snapshot) so the candidate frame can be highlighted
    /// while dragging: a node not yet in any frame doesn't affect any
    /// frame's live bounds, so there's no self-referential loop to guard
    /// against here.
    pub(crate) fn compute_drop_target_frame(
        &self,
        node_id: NodeId,
        layout: &GraphWidgetLayout,
    ) -> Option<FrameId> {
        if self
            .graph
            .frames
            .iter()
            .any(|frame| frame.node_ids.contains(&node_id))
        {
            return None;
        }
        let center = layout.nodes.get(&node_id)?.header_rect().center();
        layout
            .frame_rects
            .iter()
            .filter(|(_, rect)| rect.contains(center))
            .min_by(|(a_id, a_rect), (b_id, b_rect)| {
                a_rect
                    .area()
                    .total_cmp(&b_rect.area())
                    .then_with(|| a_id.0.cmp(&b_id.0))
            })
            .map(|(&id, _)| id)
    }

    /// Frame membership follows a drag/placement drop (Phase 1.3) — see
    /// `compute_drop_target_frame` for the rule (join-only; dragging never
    /// removes). Only called once, on gesture confirm; the changes fold
    /// into the undo snapshot the drag/placement already pushed at its
    /// start.
    pub(crate) fn resolve_frame_membership_on_drop(
        &mut self,
        node_ids: &[NodeId],
        layout: &GraphWidgetLayout,
    ) {
        if self.graph.frames.is_empty() {
            return;
        }
        let mut changed = false;
        for &node_id in node_ids {
            let Some(target_id) = self.compute_drop_target_frame(node_id, layout) else {
                continue;
            };
            if let Some(frame) = self.graph.frames.iter_mut().find(|f| f.id == target_id) {
                frame.node_ids.push(node_id);
                changed = true;
            }
        }
        if changed {
            self.graph.cleanup_frames();
        }
    }
}
