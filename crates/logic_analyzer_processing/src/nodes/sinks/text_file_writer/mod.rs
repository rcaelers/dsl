//! Text file-writer processing node.

mod facade;
#[cfg(not(target_arch = "wasm32"))]
mod implementation;
mod platform;

pub use facade::{TextFileWriterFactory, writer_factory};
#[cfg(not(target_arch = "wasm32"))]
pub use implementation::TextFileWriter;
