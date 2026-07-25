//! Built-in LogicConduit graph nodes and collected-payload presentations.

#[cfg(not(target_arch = "wasm32"))]
mod catalogs;
mod collected_payloads;
mod link;
mod nodes;
mod presentation;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(not(target_arch = "wasm32"))]
pub use catalogs::native_node_catalogs;
pub use link::link;
