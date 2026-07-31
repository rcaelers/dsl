//! CSV word-writer processing node.

mod configuration;
mod facade;
#[cfg(not(target_arch = "wasm32"))]
mod implementation;
mod platform;

pub use configuration::{CsvValueFormat, CsvWordWriterConfig};
pub use facade::create_writer;
#[cfg(not(target_arch = "wasm32"))]
pub use implementation::CsvWordWriter;
