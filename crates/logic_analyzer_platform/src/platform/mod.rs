#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
mod native_artifact_repository;
#[cfg(not(target_arch = "wasm32"))]
mod native_capture_export;
#[cfg(not(target_arch = "wasm32"))]
mod native_file_identity_cache;
#[cfg(not(target_arch = "wasm32"))]
mod native_file_source;
#[cfg(all(feature = "developer-tools", not(target_arch = "wasm32")))]
mod native_hardware_validation;
#[cfg(not(target_arch = "wasm32"))]
mod native_sigrok;
#[cfg(not(target_arch = "wasm32"))]
mod native_worker;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
mod web_artifact_repository;
#[cfg(target_arch = "wasm32")]
mod web_worker;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::standard_services;
#[cfg(target_os = "macos")]
pub use native::{dispatch_host_command, set_recent_files_listener};
#[cfg(all(feature = "developer-tools", not(target_arch = "wasm32")))]
pub use native_hardware_validation::{validate_capture_hardware, validate_fpga_hardware};
#[cfg(all(feature = "developer-tools", not(target_arch = "wasm32")))]
pub use native_sigrok::{validate_spi_chunk_boundaries, validate_spi_oracle};
#[cfg(target_arch = "wasm32")]
pub(crate) use web::{standard_services, standard_services_with_worker_urls};
#[cfg(target_arch = "wasm32")]
pub use web_worker::WebWorkerAdapter;
