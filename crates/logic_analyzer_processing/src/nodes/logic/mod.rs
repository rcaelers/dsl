//! # `logic_analyzer_processing::nodes::logic`
//!
//! ## Responsibility
//!
//! This namespace groups concrete protocol-independent stream transformation and control nodes.
//!
//! ## Child owners
//!
//! - [buffer](logic/buffer.md), [edge detector](logic/edge_detector.md), and
//!   [event control](logic/event_control.md)
//! - [event gate](logic/event_gate.md), [logic gate](logic/logic_gate.md), and
//!   [packet framer](logic/packet_framer.md)
//! - [SR latch](logic/sr_latch.md), [text formatter](logic/text_formatter.md), and
//!   [timeline marker](logic/timeline_marker.md)
//! - [trigger counter](logic/trigger_counter.md), [word field extractor](logic/word_field_extractor.md),
//!   and [word matcher](logic/word_matcher.md)
//!
//! ## Boundaries
//!
//! Each child owns one runtime transformation. Generic scheduling and ports remain in
//! `signal_capture_session`; graph-node definitions and UI presentation remain above processing.

//! Control-path logic processing nodes.
//!
//! Each child module owns one UI-independent stream transformation. Graph
//! definitions, panel controls, and host scheduling remain outside this family.

pub mod buffer;
pub mod edge_detector;
pub mod event_control;
pub mod event_gate;
pub mod logic_gate;
pub mod packet_framer;
pub mod sr_latch;
pub mod text_formatter;
pub mod timeline_marker;
pub mod trigger_counter;
pub mod word_field_extractor;
pub mod word_matcher;
