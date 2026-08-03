//! # `binary_file_writer`
//!
//! ## Responsibility
//!
//! This module owns streaming binary persistence of its configured runtime payloads.
//!
//! ## Boundaries
//!
//! It receives writable output through `OutputStorage`; it does not acquire a path, open a native
//! dialog, select a browser download, or define graph-node controls.

//! Binary file-writer processing node.
//!
//! It owns binary stream encoding and writes through the explicit output-storage
//! capability. File destination choice and graph/UI policy remain outside it.

mod configuration;
mod facade;
mod implementation;

pub use configuration::{BinaryFileWriterConfig, WriteWidth};
pub use facade::{BinaryFileWriterFactory, unavailable_writer_factory, writer_factory};
pub use implementation::BinaryFileWriter;
