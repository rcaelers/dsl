//! # `csv_word_writer`
//!
//! ## Responsibility
//!
//! This module owns CSV serialization and persistence of runtime word streams.
//!
//! ## Boundaries
//!
//! Output destination acquisition is injected through `OutputStorage`; UI file controls and platform
//! file access remain outside this processing node.

//! CSV word-writer processing node.
//!
//! It serializes generic words as CSV through explicit output storage; the host
//! supplies destinations and the graph feature owns configuration presentation.

mod configuration;
mod facade;
mod implementation;

pub use configuration::{CsvValueFormat, CsvWordWriterConfig};
pub use facade::{CsvWordWriterFactory, unavailable_writer_factory, writer_factory};
pub use implementation::CsvWordWriter;
