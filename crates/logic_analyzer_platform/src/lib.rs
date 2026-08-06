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

pub use file_dialog::{
    DroppedFileData, FileDialogFilter, FileOpenDialog, FilePickerProgress, FilePickerRequest,
    FilePickerService, FileReference, FileSaveDialog,
};
std::cfg_select! {
    target_arch = "wasm32" => {
        pub use platform::{
            BrowserDocumentHost, BrowserDownload, BrowserFileImport, WebWorkerAdapter,
            browser_worker_clients, browser_worker_dsl_file_source_factory,
            browser_worker_output_storage, browser_worker_parallelism,
            browser_worker_sigrok_file_source_factory, initialize_graph_worker_runtime,
            open_browser_artifact_repository, worker_artifact_repository,
        };
    }
    _ => {
        pub use platform::{
            NativeDocumentHost, native_app_manager_factory, native_artifact_repository,
            native_dsl_file_source_factory, native_output_storage, native_sigrok_catalog_scanner,
            native_sigrok_decoder_runtime, native_sigrok_file_source_factory,
            native_u3pro16_source_factory, native_work_executor, native_worker_operation_executor,
        };
        #[cfg(feature = "developer-tools")]
        pub use platform::{
            isolated_native_artifact_repository, validate_capture_hardware, validate_fpga_hardware,
            validate_spi_chunk_boundaries, validate_spi_oracle,
        };
    }
}
