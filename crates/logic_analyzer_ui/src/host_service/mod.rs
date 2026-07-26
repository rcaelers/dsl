//! UI-owned boundary for dialogs, document persistence, and cache commands.

#[cfg(test)]
mod architecture_tests;
mod contract;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod host_service_tests;
#[cfg(not(target_arch = "wasm32"))]
#[path = "native.rs"]
mod implementation;
#[cfg(target_arch = "wasm32")]
#[path = "wasm.rs"]
mod implementation;
#[cfg(not(target_arch = "wasm32"))]
#[path = "platform_contract_native.rs"]
mod platform_contract;
#[cfg(target_arch = "wasm32")]
#[path = "platform_contract_wasm.rs"]
mod platform_contract;

pub(crate) use contract::HostService;
pub(crate) use implementation::standard_host_service;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use platform_contract::{OpenDialog, SaveDialog};
