//! Checked single-connection routing over rectangular obstacles.
//!
//! Owns escape validation, boundary-coordinate visibility search, and final path checks.
//! Inputs are geometric records; document identity and layout adaptation stay outside.

mod contract;
mod obstacle;
mod router;
mod search;
#[cfg(test)]
mod tests;

pub(crate) use contract::{PortGeometry, PortSide, RouteConfig, RouteFailure, RouteInput};
pub(crate) use router::route;
