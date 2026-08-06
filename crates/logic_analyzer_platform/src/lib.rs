//! Target-selected adapters for services owned by reusable core crates.
//!
//! This crate is the target-selection boundary for host APIs. Core crates
//! define portable contracts, platform supplies host implementations, and
//! application roots compose those implementations with product behavior.
//!
//! The public facade exposes host-selected services and constructors.
//! Platform-neutral policy, product composition, and concrete node selection
//! remain outside this crate.

mod file_dialog;
mod platform;
mod services;

pub use file_dialog::{
    DroppedFileData, FileDialogFilter, FileOpenDialog, FilePickerProgress, FilePickerRequest,
    FilePickerService, FileReference, FileSaveDialog,
};
pub use services::{PlatformServices, WorkerGraphHostServices};

/// Builds host services for the selected target.
pub fn standard_services(application_id: &str) -> PlatformServices {
    platform::standard_services(application_id)
}

std::cfg_select! {
    target_arch = "wasm32" => {
        pub use platform::{
            BrowserDocumentHost, BrowserDownload, WebWorkerAdapter,
            initialize_graph_worker_runtime, worker_artifact_repository,
            worker_graph_host_services,
        };

        /// Builds web services with a parallel finite-operation worker pool when the
        /// browser accepts the supplied generated-module URLs.
        ///
        /// # Parameters
        /// - `module_url`: Input consumed by this operation.
        /// - `wasm_url`: Input consumed by this operation.
        pub async fn standard_services_with_worker_urls(
            application_id: &str,
            module_url: &str,
            wasm_url: &str,
        ) -> PlatformServices {
            platform::standard_services_with_worker_urls(application_id, module_url, wasm_url).await
        }
    }
    _ => {
        pub use platform::NativeDocumentHost;
        #[cfg(feature = "developer-tools")]
        pub use platform::{
            isolated_native_artifact_repository, validate_capture_hardware, validate_fpga_hardware,
            validate_spi_chunk_boundaries, validate_spi_oracle,
        };
    }
}
