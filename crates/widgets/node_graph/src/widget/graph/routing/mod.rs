//! Private connection-path geometry shared by painting and wire gestures.
//!
//! This owner stores path segments and their bounded-error interaction approximation.
//! It accepts geometry only; graph topology, styling policy, and layout adaptation belong
//! to the enclosing widget. It contains no document mutation or routing policy.

mod geometry;
mod paint;

#[cfg(test)]
pub(crate) use geometry::PathSegment;
pub(crate) use geometry::WirePath;
pub(crate) use paint::draw_path;
