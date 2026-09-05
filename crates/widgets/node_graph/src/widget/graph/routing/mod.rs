//! Private connection-path geometry shared by painting and wire gestures.
//!
//! This owner stores path segments and their bounded-error interaction approximation.
//! It accepts geometry only; graph topology, styling policy, and layout adaptation belong
//! to the enclosing widget. It contains no document mutation.

mod bundle;
mod bundle_corridor;
mod bundle_quality;
#[cfg(test)]
mod bundle_tests;
mod contract;
mod corridor;
mod geometry;
mod grouping;
mod history;
mod individual;
mod individual_quality;
mod ordered_smoothing;
#[cfg(test)]
mod ordered_smoothing_tests;
mod paint;
mod smoothing;
#[cfg(test)]
mod smoothing_tests;
mod variable_spacing;
#[cfg(test)]
mod variable_spacing_tests;

pub(crate) use bundle::BundleMember;
pub(crate) use bundle_quality::route_quality_bundle;
pub(crate) use contract::{
    PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput, WorkBudget,
};
pub(crate) use geometry::{PathSegment, WirePath};
pub(crate) use grouping::{BundleCandidate, compatible_groups};
pub(crate) use history::avoids_changed_obstacles;
pub(crate) use individual::route_with_budget;
pub(crate) use individual_quality::improve_route;
pub(crate) use paint::draw_path;
