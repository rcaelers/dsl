//! Binary file-writer processing node.

mod configuration;
mod facade;
mod implementation;

pub use configuration::{BinaryFileWriterConfig, WriteWidth};
pub use facade::{BinaryFileWriterFactory, unavailable_writer_factory, writer_factory};
pub use implementation::BinaryFileWriter;
