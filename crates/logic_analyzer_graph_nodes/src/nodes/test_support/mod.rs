//! Private fixtures for isolated concrete-node tests.

mod assertion;

pub(crate) use assertion::{
    assert_node_registration_contract, assert_node_registration_contract_with_state,
};
