#![allow(dead_code)] // Each integration-test binary consumes a different fixture subset.

mod catalog;
mod graphs;
mod host_factories;

#[allow(unused_imports)]
pub(crate) use catalog::{build_registry, node_name, node_semantics};
#[allow(unused_imports)]
pub(crate) use graphs::{build_binary_decoder_demo, build_live_binary_test, populate_startup};
#[allow(unused_imports)]
pub(crate) use host_factories::{GraphHarness, test_live_compiler, test_platform_compiler};
