//! Private fixtures for isolated concrete-node tests.

mod assertion;
mod build_context;
mod capture_index_factory;
mod platform_parity;
mod process_node;
mod sigrok_descriptor;

pub(crate) use assertion::{
    assert_node_registration_contract, assert_node_registration_contract_with_state,
    assert_node_registration_contract_without_runtime,
    assert_node_registration_contract_without_runtime_with_state,
};
pub(crate) use build_context::TestNodeBuildContext;
pub(crate) use capture_index_factory::TestCaptureIndexFactory;
pub(crate) use platform_parity::{
    PlatformParityCapabilities, PlatformParityCapabilityRegistration, TestSourceFactory,
    TestWriterFactory, platform_parity_capabilities,
};
pub(crate) use process_node::TestProcessNode;
pub(crate) use sigrok_descriptor::{test_sigrok_logic_descriptor, test_sigrok_stacked_descriptor};
