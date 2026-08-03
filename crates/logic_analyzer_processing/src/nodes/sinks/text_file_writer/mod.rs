//! # `text_file_writer`
//!
//! ## Responsibility
//!
//! This module owns line-oriented text persistence from runtime text streams.
//!
//! ## Boundaries
//!
//! It receives destination storage through `OutputStorage` and does not own graph UI, dialog behavior,
//! or host-specific output APIs.

//! Text file-writer processing node.
//!
//! It writes formatted text through injected output storage. User destination
//! selection and graph/editor presentation belong to separate owners.

mod facade;
mod implementation;

pub use facade::{TextFileWriterFactory, unavailable_writer_factory, writer_factory};
pub use implementation::TextFileWriter;
