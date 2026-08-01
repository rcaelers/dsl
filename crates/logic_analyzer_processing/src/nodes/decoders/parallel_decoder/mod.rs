//! Parallel bus decoder with host-injected execution.

mod implementation;
mod sampling_provider;
mod types;

pub use implementation::{ParallelDecoder, ParallelDecoderMetrics, ParallelDecoderMetricsSnapshot};
pub use types::{ParallelInputStrategy, StrobeMode};
