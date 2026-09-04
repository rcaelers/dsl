//! Private connection-path geometry shared by painting and wire gestures.
//!
//! This owner stores path segments and their bounded-error interaction approximation.
//! It accepts geometry only; graph topology, styling policy, and layout adaptation belong
//! to the enclosing widget. It contains no document mutation.

mod geometry;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "checked routing is activated by the next implementation step"
    )
)]
mod individual;
mod paint;

pub(crate) use geometry::{PathSegment, WirePath};
pub(crate) use individual::{PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, route};
pub(crate) use paint::draw_path;
