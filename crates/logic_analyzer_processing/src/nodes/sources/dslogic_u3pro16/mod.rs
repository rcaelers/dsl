//! # `dslogic_u3pro16`
//!
//! ## Responsibility
//!
//! This module owns the DSLogic U3Pro16 capture protocol, source state machine, and processing-source
//! behavior.
//!
//! ## Boundaries
//!
//! USB transport and FPGA-image acquisition are host capabilities injected by platform composition.
//! Graph state, editor controls, and target selection do not belong to this module.

//! DSLogic U3Pro16 source node, USB driver, and acquisition profiles.
//!
//! It owns concrete device behavior and portable configuration contracts. USB
//! transport and firmware capabilities are injected by platform composition.

mod facade;
mod transport;

pub use facade::{DsLogicU3Pro16SourceFactory, unavailable_source_factory};
pub use signal_processing::logic_analyzer::{
    CaptureMode, ClockEdge, ClockSource, LogicCaptureConfig, LogicEncodingRequest, LogicTrigger,
    LogicTriggerStage, TriggerCondition, TriggerLogic,
};
pub use transport::{DsLogicU3Pro16TransportFactory, LinkSpeed, UsbError, UsbTransport};

std::cfg_select! {
    target_arch = "wasm32" => {}
    _ => {
        mod buffered;
        mod capture;
        mod common;
        mod implementation;
        mod source;
        mod streaming;

        #[cfg(feature = "developer-tools")]
        mod benchmark;
        #[cfg(feature = "developer-tools")]
        mod hardware_validation;

        #[cfg(feature = "developer-tools")]
        pub use benchmark::run_streaming_benchmark;
        pub use capture::DsLogicU3Pro16Capture;
        #[cfg(feature = "developer-tools")]
        pub use hardware_validation::{validate_capture_hardware, validate_fpga_hardware};
        pub use source::DsLogicU3Pro16Source;
    }
}
