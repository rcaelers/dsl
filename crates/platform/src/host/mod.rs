std::cfg_select! {
    target_arch = "wasm32" => {
        mod web;
        mod web_artifact_repository;
        mod web_document;
        #[allow(unreachable_pub)]
        mod web_worker;

        pub use web::{browser_worker_parallelism, open_browser_artifact_repository};
        pub use web_document::{
            BrowserDocumentHost, BrowserDownload, BrowserDownloadFile, queue_browser_downloads,
        };
        pub use web_worker::WebWorkerAdapter;
    }
    _ => {
        mod native;
        mod native_artifact_repository;
        mod native_file_output;
        mod native_file_source;
        mod native_document;
        mod native_usb;
        mod native_worker;

        pub use native::{
            native_artifact_repository, native_work_executor, native_worker_operation_executor,
        };
        #[cfg(feature = "developer-tools")]
        pub use native_artifact_repository::isolated_native_artifact_repository;
        pub use native_document::NativeDocumentHost;
        pub use native_file_output::{
            native_append_file, native_create_file, native_create_parent_directories,
            native_path_exists,
        };
        pub use native_file_source::native_file_byte_source;
        pub use native_usb::{
            NativeUsbDevice, NativeUsbDeviceSelector, UsbDeviceOpenError, UsbDeviceOpenOperation,
            UsbLinkSpeed, UsbTransferError,
        };
    }
}
