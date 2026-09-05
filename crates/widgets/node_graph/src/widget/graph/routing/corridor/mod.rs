//! Shared rectangular-obstacle validation, port escapes, and visibility search.

mod curve;
mod obstacle;
mod search;

pub(crate) use curve::cubic_clear;
pub(crate) use obstacle::{clear, escape, expanded};
pub(crate) use search::Channels;
