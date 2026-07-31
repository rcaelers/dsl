//! Binary file-writer processing node.

mod configuration;
mod facade;
#[cfg(not(target_arch = "wasm32"))]
mod implementation;
mod platform;

pub use configuration::{BinaryFileWriterConfig, WriteWidth};
pub use facade::{BinaryFileWriterFactory, writer_factory};
#[cfg(not(target_arch = "wasm32"))]
pub use implementation::BinaryFileWriter;
