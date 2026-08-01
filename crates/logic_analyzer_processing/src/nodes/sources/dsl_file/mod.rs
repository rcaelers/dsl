//! DSL capture-file source node.

mod configuration;
mod facade;
#[cfg(not(target_arch = "wasm32"))]
mod implementation;

pub use configuration::DslFileSourceConfig;
pub use facade::{DslFileSourceFactory, unavailable_source_factory};
#[cfg(not(target_arch = "wasm32"))]
pub use implementation::DslFileSource;
