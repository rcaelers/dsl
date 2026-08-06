std::cfg_select! {
    target_arch = "wasm32" => {
        mod web;
        mod web_artifact_repository;
        #[allow(unreachable_pub)]
        mod web_capture_worker;
        mod web_document;
        mod web_file_import;
        mod web_output_storage;
        #[allow(unreachable_pub)]
        mod web_worker;

        pub use web::{
            BrowserFileImport, browser_worker_clients, browser_worker_dsl_file_source_factory,
            browser_worker_output_storage, browser_worker_parallelism,
            browser_worker_sigrok_file_source_factory, open_browser_artifact_repository,
        };
        pub use web_capture_worker::{initialize_graph_worker_runtime, worker_artifact_repository};
        pub use web_document::{BrowserDocumentHost, BrowserDownload};
        pub use web_worker::WebWorkerAdapter;
    }
    _ => {
        mod native;
        mod native_artifact_repository;
        mod native_file_identity_cache;
        mod native_file_source;
        mod native_document;
        mod native_sigrok;
        mod native_worker;

        #[cfg(feature = "developer-tools")]
        mod native_hardware_validation;

        pub use native::{
            native_app_manager_factory, native_artifact_repository, native_dsl_file_source_factory,
            native_output_storage, native_sigrok_catalog_scanner, native_sigrok_decoder_runtime,
            native_sigrok_file_source_factory, native_u3pro16_source_factory, native_work_executor,
            native_worker_operation_executor,
        };
        #[cfg(feature = "developer-tools")]
        pub use native_artifact_repository::isolated_native_artifact_repository;
        pub use native_document::NativeDocumentHost;
        #[cfg(feature = "developer-tools")]
        pub use native_hardware_validation::{validate_capture_hardware, validate_fpga_hardware};
        #[cfg(feature = "developer-tools")]
        pub use native_sigrok::{validate_spi_chunk_boundaries, validate_spi_oracle};
    }
}
