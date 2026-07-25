#[cfg_attr(target_arch = "wasm32", path = "builder_wasm.rs")]
mod builder;
mod definition;
#[cfg_attr(target_arch = "wasm32", path = "metadata_platform_wasm.rs")]
mod metadata_platform;
mod registration;
