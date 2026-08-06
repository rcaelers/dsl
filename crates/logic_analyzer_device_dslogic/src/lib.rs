//! DSLogic device acquisition protocols and processing sources.
//!
//! This crate owns DSLogic U3Pro16 behavior and consumes injected USB transport and FPGA-image
//! capabilities. It does not select a host adapter, define graph nodes, or own UI policy.

#[cfg(test)]
mod architecture_tests;
mod device;

pub use device::{
    DsLogicU3Pro16SourceFactory, DsLogicU3Pro16TransportFactory, LinkSpeed, UsbError, UsbTransport,
    unavailable_source_factory,
};

std::cfg_select! {
    target_arch = "wasm32" => {}
    _ => {
        #[cfg(feature = "developer-tools")]
        pub use device::run_streaming_benchmark;
        pub use device::{DsLogicU3Pro16Capture, DsLogicU3Pro16Source, source_factory};
        #[cfg(feature = "developer-tools")]
        pub use device::{validate_capture_hardware, validate_fpga_hardware};
    }
}
