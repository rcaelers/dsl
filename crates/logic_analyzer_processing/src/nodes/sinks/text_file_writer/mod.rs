//! Text file-writer processing node.

mod facade;
mod implementation;

pub use facade::{TextFileWriterFactory, unavailable_writer_factory, writer_factory};
pub use implementation::TextFileWriter;
