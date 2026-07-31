//! Data-persistence processing nodes.

pub mod binary_file_writer;
pub mod csv_word_writer;
pub mod discard_writer;
pub mod text_file_writer;
pub mod tgck_recorder;

#[cfg(not(target_arch = "wasm32"))]
mod output_storage;
