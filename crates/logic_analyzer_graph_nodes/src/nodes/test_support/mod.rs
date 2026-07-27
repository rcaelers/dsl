//! Private fixtures for isolated concrete-node tests.

mod assertion;
mod build_context;

pub(crate) use assertion::{
    assert_node_registration_contract, assert_node_registration_contract_with_state,
    assert_node_registration_contract_without_runtime,
};
pub(crate) use build_context::TestNodeBuildContext;
