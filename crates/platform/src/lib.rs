//! Target-selected adapters for services owned by reusable core crates.
//!
//! This crate is the target-selection boundary for host APIs. Core crates
//! define portable contracts, platform supplies host implementations, and
//! application roots compose those implementations with product behavior.
//!
//! The public facade exposes host-selected services and constructors.
//! Platform-neutral policy, product composition, and concrete node selection
//! remain outside this crate.

mod document;
mod file_dialog;
mod host;

pub use document::DocumentError;
pub use file_dialog::{
    DroppedFileData, FileDialogFilter, FileOpenDialog, FilePickerError, FilePickerProgress,
    FilePickerRequest, FilePickerService, FileReference, FileSaveDialog,
};
std::cfg_select! {
    target_arch = "wasm32" => {
        pub use host::{
            BrowserDocumentHost, BrowserDownload, BrowserDownloadFile, WebWorkerAdapter,
            browser_worker_parallelism, open_browser_artifact_repository, queue_browser_downloads,
        };
    }
    _ => {
        #[cfg(feature = "developer-tools")]
        pub use host::isolated_native_artifact_repository;
        pub use host::{
            NativeDocumentHost, NativeUsbDevice, NativeUsbDeviceSelector, UsbLinkSpeed,
            UsbTransferError, native_append_file, native_artifact_repository, native_create_file,
            native_create_parent_directories, native_file_byte_source, native_path_exists,
            native_work_executor, native_worker_operation_executor,
        };
    }
}
