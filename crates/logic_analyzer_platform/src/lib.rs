//! Target-selected adapters for services owned by reusable core crates.
//!
//! This crate is the composition boundary for host APIs. Core crates define
//! portable contracts and receive their implementations from application
//! roots; they never depend on this crate.
//!
//! The public facade exposes only the opaque service bundle and constructors.
//! Platform-neutral policy and data models remain in the core crates that own
//! their capability contracts.

mod platform;
mod services;

#[cfg(test)]
mod architecture_tests;

pub use services::PlatformServices;

/// Builds the services appropriate for the selected application host.
pub fn standard_services() -> PlatformServices {
    platform::standard_services()
}

std::cfg_select! {
    target_arch = "wasm32" => {
        pub use platform::WebWorkerAdapter;

        /// Builds web services with a parallel finite-operation worker pool when the
        /// browser accepts the supplied generated-module URLs.
        ///
        /// # Parameters
        /// - `module_url`: Input consumed by this operation.
        /// - `wasm_url`: Input consumed by this operation.
        pub async fn standard_services_with_worker_urls(
            module_url: &str,
            wasm_url: &str,
        ) -> PlatformServices {
            platform::standard_services_with_worker_urls(module_url, wasm_url).await
        }
    }
    _ => {
        #[cfg(target_os = "macos")]
        pub use platform::{dispatch_host_command, set_recent_files_listener};
        #[cfg(feature = "developer-tools")]
        pub use platform::{
            isolated_native_artifact_repository, validate_capture_hardware, validate_fpga_hardware,
            validate_spi_chunk_boundaries, validate_spi_oracle,
        };
    }
}
