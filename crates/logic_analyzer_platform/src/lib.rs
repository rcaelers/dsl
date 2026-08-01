//! Target-selected adapters for services owned by reusable core crates.
//!
//! This crate is the composition boundary for host APIs. Core crates define
//! portable contracts and receive their implementations from application
//! roots; they never depend on this crate.

mod platform;
mod services;

#[cfg(target_arch = "wasm32")]
pub use platform::WebWorkerAdapter;
#[cfg(target_os = "macos")]
pub use platform::{dispatch_host_command, set_recent_files_listener};
#[cfg(all(feature = "developer-tools", not(target_arch = "wasm32")))]
pub use platform::{
    validate_capture_hardware, validate_fpga_hardware, validate_spi_chunk_boundaries,
    validate_spi_oracle,
};
pub use services::PlatformServices;

/// Builds the services appropriate for the selected application host.
pub fn standard_services() -> PlatformServices {
    platform::standard_services()
}

/// Builds web services with a parallel finite-operation worker pool when the
/// browser accepts the supplied generated-module URLs.
#[cfg(target_arch = "wasm32")]
pub fn standard_services_with_worker_urls(module_url: &str, wasm_url: &str) -> PlatformServices {
    platform::standard_services_with_worker_urls(module_url, wasm_url)
}
