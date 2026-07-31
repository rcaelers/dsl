//! Deterministic synthetic source and its native acquisition provider.

mod implementation;
mod presentation;

pub use implementation::SyntheticCaptureSource;
pub(crate) use presentation::synthetic_presentation;
