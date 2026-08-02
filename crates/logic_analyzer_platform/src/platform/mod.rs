std::cfg_select! {
    target_arch = "wasm32" => {
        mod web;
        mod web_artifact_repository;
        mod web_file_import;
        mod web_worker;

        pub(crate) use web::{standard_services, standard_services_with_worker_urls};
        pub use web_worker::WebWorkerAdapter;
    }
    _ => {
        mod native;
        mod native_artifact_repository;
        mod native_capture_export;
        mod native_file_identity_cache;
        mod native_file_source;
        mod native_sigrok;
        mod native_worker;

        #[cfg(feature = "developer-tools")]
        mod native_hardware_validation;

        pub(crate) use native::standard_services;
        #[cfg(target_os = "macos")]
        pub use native::{dispatch_host_command, set_recent_files_listener};
        #[cfg(feature = "developer-tools")]
        pub use native_hardware_validation::{validate_capture_hardware, validate_fpga_hardware};
        #[cfg(feature = "developer-tools")]
        pub use native_sigrok::{validate_spi_chunk_boundaries, validate_spi_oracle};
    }
}
