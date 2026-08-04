//! Neutral immutable processing-graph contract shared by graph producers and consumers.

mod plan;

#[cfg(test)]
mod architecture_tests;

pub use plan::*;
