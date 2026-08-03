//! # `signal_processing::waveform_index`
//!
//! ## Responsibility
//!
//! This module owns finite and growing generic waveform indexes plus bounded sampled-window queries.
//!
//! ## Boundaries
//!
//! Capture sources provide packed samples and identities. The module does not acquire files, choose
//! storage locations, render waveforms, or decide which graph source is visible.

//! Generic waveform-summary indexing and sampling.
//!
//! The index turns capture data into queryable summaries without knowing a concrete
//! source format, protocol, viewer widget, or host storage implementation.

mod builder;
mod exact;
mod growing;
mod query;
mod reader;
mod resolution;
mod storage;
mod types;

pub use exact::exact_window_sample_limit;
pub use growing::{GrowingCaptureIndex, GrowingCaptureIndexWorker};
pub use reader::IndexSampler;
pub use types::CaptureIndexProgress;
