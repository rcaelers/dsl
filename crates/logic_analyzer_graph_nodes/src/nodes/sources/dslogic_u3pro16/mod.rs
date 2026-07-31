mod builder;
mod capture_configuration;
mod definition;
#[cfg(not(target_arch = "wasm32"))]
mod implementation;
#[cfg_attr(target_arch = "wasm32", path = "live_capture_wasm.rs")]
mod live_capture;
mod live_edit;
#[cfg_attr(target_arch = "wasm32", path = "presentation_platform_wasm.rs")]
mod presentation_platform;
mod registration;
mod trigger;
mod trigger_lowering;
