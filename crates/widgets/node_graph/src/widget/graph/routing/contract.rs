use egui::{Pos2, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PortSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PortGeometry {
    /// Index into this input's rectangles, not a persisted node identity.
    pub(crate) obstacle: usize,
    pub(crate) position: Pos2,
    pub(crate) side: PortSide,
}

pub(crate) struct RouteInput<'a> {
    pub(crate) nodes: &'a [Rect],
    pub(crate) source: PortGeometry,
    pub(crate) target: PortGeometry,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct RouteConfig {
    pub(crate) clearance_x: f32,
    pub(crate) clearance_y: f32,
    pub(crate) escape: f32,
    pub(crate) safety: f32,
    pub(crate) lane_spacing: f32,
    pub(crate) preferred_lane_spacing: f32,
    pub(crate) corner_radius: f32,
    pub(crate) max_smoothing_work: usize,
    pub(crate) bend_cost: f64,
    pub(crate) vertical_weight: f64,
    pub(crate) max_vertices: usize,
    pub(crate) max_work: usize,
}

impl Default for RouteConfig {
    fn default() -> Self {
        Self {
            clearance_x: 20.0,
            clearance_y: 16.0,
            escape: 30.0,
            safety: 0.01,
            lane_spacing: 8.0,
            preferred_lane_spacing: 12.0,
            corner_radius: 12.0,
            max_smoothing_work: 50_000,
            bend_cost: 20.0,
            vertical_weight: 1.5,
            max_vertices: 100_000,
            max_work: 4_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteFailure {
    InvalidGeometry,
    BlockedEscape,
    NoCorridor,
    WorkLimit,
}

pub(crate) struct WorkBudget {
    remaining: usize,
}

impl WorkBudget {
    pub(crate) fn new(remaining: usize) -> Self {
        Self { remaining }
    }
    pub(crate) fn spend(&mut self, count: usize) -> Result<(), RouteFailure> {
        self.remaining = self
            .remaining
            .checked_sub(count)
            .ok_or(RouteFailure::WorkLimit)?;
        Ok(())
    }
}
