#[cfg_attr(target_arch = "wasm32", path = "builder_wasm.rs")]
mod builder;
mod capture_configuration;
mod definition;
#[cfg(not(target_arch = "wasm32"))]
mod implementation;
#[cfg(not(target_arch = "wasm32"))]
mod live_capture;
mod live_edit;
mod registration;
mod trigger;
mod trigger_lowering;
