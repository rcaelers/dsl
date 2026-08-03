//! # `parallel_decoder`
//!
//! ## Responsibility
//!
//! This module owns parallel-bus sampling, assembly, and decoded-word production for its configured
//! clock, data, and qualifier inputs.
//!
//! ## Boundaries
//!
//! It does not own graph socket definitions, display formatting, or host execution selection. Those
//! concerns remain with graph-node, UI, and platform owners.

//! Parallel bus decoder with host-injected execution.
//!
//! It decodes generic sampled parallel inputs using its explicit configuration;
//! host execution and graph presentation are supplied outside this runtime node.

mod implementation;
mod sampling_provider;
mod types;

pub use implementation::{ParallelDecoder, ParallelDecoderMetrics, ParallelDecoderMetricsSnapshot};
pub use types::{ParallelInputStrategy, StrobeMode};
