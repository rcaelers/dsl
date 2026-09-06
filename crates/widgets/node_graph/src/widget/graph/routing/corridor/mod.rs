//! Shared rectangular-obstacle validation, port escapes, and visibility search.

mod curve;
mod obstacle;
mod search;

pub(crate) use curve::cubic_clear;
pub(crate) use obstacle::{ObstacleSubset, clear, escape, expand_obstacle, expanded};
pub(crate) use search::{Channels, parallel_overlap};
