//! Validation failures for driver-neutral live-capture values.

use thiserror::Error;

use signal_capture::CaptureChannelId;

/// An invariant violation while constructing a driver-neutral live-capture value.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum CaptureValidationError {
    /// A supported setting tuple did not contain a physical channel.
    #[error("a capture setting combination requires at least one channel")]
    SettingChannelsEmpty,
    /// A supported setting tuple did not contain a sample rate.
    #[error("a capture setting combination requires at least one sample rate")]
    SettingSampleRatesEmpty,
    /// A supported setting tuple contained the invalid zero sample rate.
    #[error("capture setting sample rates must be non-zero")]
    SettingSampleRateZero,
    /// A physical channel appeared more than once in one supported tuple.
    #[error("capture setting channel '{channel}' is configured more than once")]
    SettingChannelDuplicate {
        /// Repeated physical-channel identity.
        channel: CaptureChannelId,
    },
    /// A sample rate appeared more than once in one supported tuple.
    #[error("capture setting sample rate {sample_rate_hz} Hz is configured more than once")]
    SettingSampleRateDuplicate {
        /// Repeated sample rate in hertz.
        sample_rate_hz: u64,
    },
    /// A provider did not expose any supported setting tuple.
    #[error("capture capabilities require a non-empty setting matrix")]
    CapabilitySettingMatrixEmpty,
    /// An analysis source did not map any physical channel.
    #[error("live analysis requires at least one channel")]
    AnalysisChannelsEmpty,
    /// An analysis source received a non-finite or non-positive sample rate.
    #[error("live analysis sample rate {sample_rate_hz} Hz must be finite and positive")]
    AnalysisSampleRateInvalid {
        /// Rejected sample rate in hertz.
        sample_rate_hz: f64,
    },
    /// A sample rate cannot be represented by the analysis timestamp unit.
    #[error("live analysis sample rate {sample_rate_hz} Hz cannot be represented by SampleBlock")]
    AnalysisTimestampStepUnrepresentable {
        /// Rejected sample rate in hertz.
        sample_rate_hz: f64,
    },
    /// A physical channel appeared in multiple analysis mappings.
    #[error("live analysis channel '{channel}' is configured more than once")]
    AnalysisChannelDuplicate {
        /// Repeated physical-channel identity.
        channel: CaptureChannelId,
    },
    /// An analysis mapping contained an empty output-port name.
    #[error("live analysis channel '{channel}' has an empty output port name")]
    AnalysisPortNameEmpty {
        /// Physical channel owning the invalid mapping.
        channel: CaptureChannelId,
    },
    /// Multiple analysis representations claimed the same output-port name.
    #[error("live analysis output port '{port}' is configured more than once")]
    AnalysisPortDuplicate {
        /// Repeated graph output-port name.
        port: String,
    },
}
