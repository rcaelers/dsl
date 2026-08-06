//! # `dsl_file`
//!
//! ## Responsibility
//!
//! This module owns DSL archive parsing, prepared-source indexing, and replay processing behavior.
//!
//! ## Boundaries
//!
//! Host path acquisition is an explicitly allowlisted compatibility boundary; normal composition
//! injects prepared byte sources. Graph definitions, file dialogs, and viewer attachment remain above
//! this processing source.

//! DSL capture-file source node.
//!
//! This module owns DSL parsing and source construction after a host supplies a
//! prepared byte source. File picking, paths, and graph presentation are separate.

mod configuration;
mod cooperative;
mod facade;
mod implementation;
mod path_compatibility;
mod prepared_file;

pub use configuration::DslFileSourceConfig;
pub use facade::{DslFileSourceFactory, unavailable_source_factory};
pub use implementation::DslFileSource;
pub use prepared_file::prepared_file_source_factory;
