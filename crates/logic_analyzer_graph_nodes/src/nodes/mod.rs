//! Concrete graph-node definitions, builders, registrations, and test fixtures.

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
