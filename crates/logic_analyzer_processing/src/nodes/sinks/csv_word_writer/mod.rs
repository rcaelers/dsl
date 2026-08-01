//! CSV word-writer processing node.

mod configuration;
mod facade;
mod implementation;

pub use configuration::{CsvValueFormat, CsvWordWriterConfig};
pub use facade::{CsvWordWriterFactory, unavailable_writer_factory, writer_factory};
pub use implementation::CsvWordWriter;
