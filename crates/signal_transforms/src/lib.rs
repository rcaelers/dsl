//! Portable, UI-independent signal stream transformations.
//!
//! Each public module owns one transform. Runtime scheduling belongs to `signal_runtime`; graph
//! definitions and presentation remain outside this crate.

pub mod buffer;
pub mod edge_detector;
pub mod event_control;
pub mod event_counter;
pub mod event_gate;
pub mod logic_gate;
pub mod sr_latch;
pub mod text_formatter;
pub mod timeline_marker;
pub mod word_field_extractor;
pub mod word_matcher;
