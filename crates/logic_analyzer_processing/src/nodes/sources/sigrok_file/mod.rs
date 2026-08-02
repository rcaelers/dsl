//! Sigrok capture-file source node.

mod configuration;
mod facade;
mod implementation;
mod path_compatibility;

pub use configuration::SigrokFileSourceConfig;
pub use facade::{SigrokFileSourceFactory, portable_source_factory};
pub use implementation::SigrokFileSource;
