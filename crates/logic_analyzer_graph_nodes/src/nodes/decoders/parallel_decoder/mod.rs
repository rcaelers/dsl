//! Parallel-bus decoder graph node.

mod builder;
mod definition;
mod presentation;
mod registration;

pub(crate) use builder::ParallelDecoderBuilder;
pub(crate) use definition::{ParallelDecoder, ParallelDecoderState};
