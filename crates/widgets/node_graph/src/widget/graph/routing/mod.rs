//! Private connection-path geometry shared by painting and wire gestures.
//!
//! This owner stores path segments and their bounded-error interaction approximation.
//! It accepts geometry only; graph topology, styling policy, and layout adaptation belong
//! to the enclosing widget. It contains no document mutation.

mod bundle;
mod bundle_corridor;
#[cfg(test)]
mod bundle_tests;
mod contract;
mod corridor;
mod geometry;
mod grouping;
mod individual;
mod paint;

pub(crate) use bundle::{BundleMember, route_bundle};
pub(crate) use contract::{
    PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget,
};
pub(crate) use geometry::{PathSegment, WirePath};
pub(crate) use grouping::{BundleCandidate, compatible_groups};
pub(crate) use individual::route_with_budget;
pub(crate) use paint::draw_path;
