//! Sigrok capture-file source node.

mod configuration;
mod facade;
#[cfg(not(target_arch = "wasm32"))]
mod implementation;
mod platform;

pub use configuration::SigrokFileSourceConfig;
pub use facade::create_source;
#[cfg(not(target_arch = "wasm32"))]
pub use implementation::SigrokFileSource;
