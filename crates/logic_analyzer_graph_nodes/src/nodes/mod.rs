//! Concrete graph-node definitions, builders, registrations, and test fixtures.

#[cfg(all(test, not(target_arch = "wasm32")))]
mod platform_registration_tests;
#[cfg(all(test, target_arch = "wasm32"))]
mod platform_registration_web_tests;
#[cfg(any(test, feature = "test-support"))]
mod test_graphs;
#[cfg(test)]
mod test_support;

pub(crate) mod decoders;
mod logic;
mod sinks;
mod sources;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use test_graphs::test_graphs_tests;
#[cfg(test)]
pub(crate) use test_support::node_name;
