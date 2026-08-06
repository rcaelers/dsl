//! Portable terminal signal consumers and output encodings.
//!
//! Destination acquisition is injected through [`OutputStorage`]. Graph lifecycle, UI policy, and
//! target-specific file access remain outside this crate.

#[cfg(test)]
mod architecture_tests;
pub mod binary_file_writer;
pub mod csv_word_writer;
pub mod discard_writer;
mod output_storage;
pub mod text_file_writer;
pub mod tgck_recorder;

pub use output_storage::{OutputFile, OutputOrigin, OutputStorage};
