//! # `sigrok_file`
//!
//! ## Responsibility
//!
//! This module owns Sigrok archive parsing, prepared-source indexing, and replay processing behavior.
//!
//! ## Boundaries
//!
//! Host path acquisition is an explicitly allowlisted compatibility boundary; normal composition
//! supplies prepared sources. Sigrok graph-node configuration and UI presentation remain elsewhere.

//! Sigrok capture-file source node.
//!
//! It owns Sigrok parsing and source construction from prepared data; host file
//! acquisition and graph/UI configuration are intentionally separate.

mod configuration;
mod cooperative;
mod facade;
mod implementation;
mod path_compatibility;
mod prepared_file;

pub use configuration::SigrokFileSourceConfig;
pub use facade::{SigrokFileSourceFactory, portable_source_factory};
pub use implementation::SigrokFileSource;
pub use prepared_file::prepared_file_source_factory;
