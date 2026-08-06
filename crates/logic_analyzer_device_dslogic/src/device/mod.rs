//! DSLogic device implementations.

mod dslogic_u3pro16;

pub use dslogic_u3pro16::{
    DsLogicU3Pro16SourceFactory, DsLogicU3Pro16TransportFactory, LinkSpeed, UsbError, UsbTransport,
    unavailable_source_factory,
};

std::cfg_select! {
    target_arch = "wasm32" => {}
    _ => {
        #[cfg(feature = "developer-tools")]
        pub use dslogic_u3pro16::run_streaming_benchmark;
        pub use dslogic_u3pro16::{DsLogicU3Pro16Capture, DsLogicU3Pro16Source, source_factory};
        #[cfg(feature = "developer-tools")]
        pub use dslogic_u3pro16::{validate_capture_hardware, validate_fpga_hardware};
    }
}
