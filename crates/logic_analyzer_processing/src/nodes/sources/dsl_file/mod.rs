//! DSL capture-file source node.

mod configuration;
mod cooperative;
mod facade;
mod implementation;
mod path_compatibility;

pub use configuration::DslFileSourceConfig;
pub use facade::{DslFileSourceFactory, unavailable_source_factory};
pub use implementation::DslFileSource;
