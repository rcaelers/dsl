//! Shared rectangular-obstacle validation, port escapes, and visibility search.

mod obstacle;
mod search;

pub(crate) use obstacle::{clear, escape, expanded};
pub(crate) use search::Channels;
