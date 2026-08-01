//! Data-persistence processing nodes.

pub mod binary_file_writer;
pub mod csv_word_writer;
pub mod discard_writer;
pub mod text_file_writer;
pub mod tgck_recorder;

mod output_storage;

pub use output_storage::{OutputFile, OutputStorage};
