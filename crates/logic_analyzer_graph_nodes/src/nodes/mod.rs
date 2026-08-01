//! Concrete graph-node definitions, builders, registrations, and test fixtures.

#[cfg(test)]
mod platform_parity_tests;
#[cfg(test)]
mod test_support;

pub(crate) mod decoders;
mod logic;
mod sinks;
pub(crate) mod sources;
