//! Private connection-path geometry shared by painting and wire gestures.
//!
//! This owner stores path segments and their bounded-error interaction approximation.
//! It accepts geometry only; graph topology, styling policy, and layout adaptation belong
//! to the enclosing widget. It contains no document mutation.

mod geometry;
mod individual;
mod paint;

pub(crate) use geometry::{PathSegment, WirePath};
pub(crate) use individual::{
    PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget, route_with_budget,
};
pub(crate) use paint::draw_path;
