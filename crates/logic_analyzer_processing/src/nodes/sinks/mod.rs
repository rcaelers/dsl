//! # `logic_analyzer_processing::nodes::sinks`
//!
//! ## Responsibility
//!
//! This namespace groups concrete terminal processing nodes that persist, discard, or record streams.
//!
//! ## Child owners
//!
//! - [binary writer](sinks/binary_file_writer.md), [CSV word writer](sinks/csv_word_writer.md), and
//!   [discard writer](sinks/discard_writer.md)
//! - [output storage contract](sinks/output_storage.md), [text writer](sinks/text_file_writer.md), and
//!   [TGCK recorder](sinks/tgck_recorder.md)
//!
//! ## Boundaries
//!
//! Each sink owns output semantics and format behavior. Destination acquisition and target-specific
//! file access are supplied through host-injected storage contracts.

//! Data-persistence processing nodes.
//!
//! These nodes consume concrete stream output and write it through injected or
//! explicit storage contracts. They do not own graph lifecycle or UI policy.

pub mod binary_file_writer;
pub mod csv_word_writer;
pub mod discard_writer;
pub mod text_file_writer;
pub mod tgck_recorder;

mod output_storage;

pub use output_storage::{OutputFile, OutputOrigin, OutputStorage};
