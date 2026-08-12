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

mod archive_work_attribution;
mod configuration;
mod cooperative;
mod facade;
mod path_compatibility;
mod prepared_file;
mod source;

pub(crate) use archive_work_attribution::{
    ArchiveWorkPhase, ArchiveWorkRecorder, active_archive_work,
};
pub use archive_work_attribution::{
    DslArchiveWorkAttribution, DslArchiveWorkCounters, DslArchiveWorkProfile,
};
pub use configuration::DslFileSourceConfig;
pub use facade::{DslFileSourceFactory, unavailable_source_factory};
pub use prepared_file::prepared_file_source_factory;
pub use source::DslFileSource;
