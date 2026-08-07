//! Modal graph interaction state and drag constraints.
//!
//! This module owns transient gesture variants and the pure coordinate rules
//! for constrained node dragging. It contains no per-frame response data,
//! graph mutation, menu policy, or gesture dispatch.

use std::collections::HashSet;
use std::rc::Rc;

use egui::{Pos2, Vec2};

use crate::model::{FrameId, NodeId, SocketId};

pub(crate) const WIRE_SNAP_DISTANCE: f32 = 18.0;
/// Ctrl-held grid size while dragging a node (Phase 6.3), in canvas units.
const GRID_SNAP: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragAxis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DragConstraint {
    pub(crate) axis: DragAxis,
    pub(crate) locked_coordinate: f32,
}

/// Nearest `grid`-unit canvas grid point to `pos`.
pub(crate) fn snap_to_grid(pos: Pos2, grid: f32) -> Pos2 {
    Pos2::new((pos.x / grid).round() * grid, (pos.y / grid).round() * grid)
}

pub(crate) fn toggle_drag_axis(
    current: Option<DragConstraint>,
    requested: DragAxis,
    position: Pos2,
) -> Option<DragConstraint> {
    if current.is_some_and(|constraint| constraint.axis == requested) {
        None
    } else {
        Some(DragConstraint {
            axis: requested,
            locked_coordinate: match requested {
                DragAxis::X => position.y,
                DragAxis::Y => position.x,
            },
        })
    }
}

pub(crate) fn constrain_drag_position(
    mut position: Pos2,
    constraint: Option<DragConstraint>,
    snap: bool,
) -> Pos2 {
    match constraint {
        Some(DragConstraint {
            axis: DragAxis::X,
            locked_coordinate,
        }) => position.y = locked_coordinate,
        Some(DragConstraint {
            axis: DragAxis::Y,
            locked_coordinate,
        }) => position.x = locked_coordinate,
        None => {}
    }
    if snap {
        match constraint.map(|constraint| constraint.axis) {
            Some(DragAxis::X) => position.x = snap_to_grid(position, GRID_SNAP).x,
            Some(DragAxis::Y) => position.y = snap_to_grid(position, GRID_SNAP).y,
            None => position = snap_to_grid(position, GRID_SNAP),
        }
    }
    position
}

pub(crate) fn rebase_drag_offset(pointer: Pos2, node_position: Pos2) -> Vec2 {
    pointer - node_position
}

#[derive(Default)]
pub(crate) enum InteractionState {
    #[default]
    Idle,
    DraggingNode {
        node_id: NodeId,
        offset: Vec2,
        constraint: Option<DragConstraint>,
    },
    DraggingFrame {
        frame_id: FrameId,
        last_canvas: Pos2,
    },
    DraggingWire {
        from: SocketId,
        from_canvas: Pos2,
        current_canvas: Pos2,
        /// Reconnects an input wire that was detached when the drag began.
        restore_on_cancel: bool,
        /// Every node with at least one socket compatible with `from` —
        /// computed once when the drag starts (`connectable_nodes`), not
        /// per frame. `render.rs` dims everything else during the drag
        /// (Phase 4.3) so viable targets pop at any zoom.
        connectable: Rc<HashSet<NodeId>>,
    },
    Panning {
        last_screen: Pos2,
    },
    BoxSelecting {
        start_canvas: Pos2,
        current_canvas: Pos2,
    },
    /// Ctrl+right-drag draws a freeform path; wires it crosses are cut on release.
    CuttingWire {
        path: Vec<Pos2>,
    },
    /// Freshly added/duplicated/pasted nodes follow the pointer until a
    /// primary-button click confirms placement (Phase 1.2), mirroring
    /// Blender's grab-on-add. Escape/secondary-click cancels by undoing the
    /// snapshot taken when the gesture started. `anchor_canvas` is the
    /// pointer position as of the last processed frame — movement is a
    /// per-frame delta from it, not a fixed offset from gesture start.
    PlacingNodes {
        anchor_canvas: Pos2,
        /// True only for the first `update_placing_nodes` tick after this
        /// state is entered — the same input frame that processed the
        /// triggering click (e.g. clicking a node type in the Add menu).
        /// After an idle frame, egui can deliver a mouse press *and*
        /// release together in one input frame; without this guard, that
        /// same fused event would immediately satisfy the primary-button
        /// confirm check and end placement before the user ever gets to
        /// move the node. Keyboard-triggered entries (Shift+D, Ctrl+V)
        /// don't hit this, since no pointer button is involved at all.
        just_entered: bool,
    },
}

impl InteractionState {
    pub(crate) fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    pub(crate) fn use_fast_rendering(&self) -> bool {
        matches!(
            self,
            Self::Panning { .. }
                | Self::DraggingNode { .. }
                | Self::DraggingFrame { .. }
                | Self::PlacingNodes { .. }
        )
    }
}
