//! U3Pro16 live-capture setup and preparation.

use std::sync::Arc;

use signal_capture_session::logic_analyzer::{CaptureMode, LogicCaptureConfig};
use signal_capture_session::{
    AcquisitionContext, AcquisitionError, AcquisitionResult, CaptureChannelId, CaptureDataDelivery,
    CaptureStartMode, ConfiguredAcquisition, PreparedAcquisition,
};

use super::buffered::BufferedProvider;
use super::implementation::DsLogicCapturePlan;
use super::streaming::StreamingProvider;
use super::transport::{DsLogicU3Pro16TransportFactory, LinkSpeed};

#[derive(Clone, Copy)]
enum CaptureProfile {
    Buffered,
    Streaming,
}

/// A configured U3Pro16 live capture.
///
/// This is the concrete acquisition counterpart to [`super::DsLogicU3Pro16Source`].
/// It owns all device-profile selection; callers only configure it, obtain its
/// generic capture facts, and prepare it through the acquisition runtime.
#[derive(Clone)]
pub struct DsLogicU3Pro16Capture {
    config: LogicCaptureConfig,
    channels: Arc<[CaptureChannelId]>,
    profile: CaptureProfile,
    capture_window_samples: u64,
    transport_factory: Arc<dyn DsLogicU3Pro16TransportFactory>,
}

impl DsLogicU3Pro16Capture {
    /// Validates a U3Pro16 capture request without opening the device.
    ///
    /// # Parameters
    /// - `config`: Input consumed by this operation.
    /// - `channels`: Input consumed by this operation.
    /// - `transport_factory`: Input consumed by this operation.
    pub fn new(
        config: LogicCaptureConfig,
        channels: impl Into<Arc<[CaptureChannelId]>>,
        transport_factory: Arc<dyn DsLogicU3Pro16TransportFactory>,
    ) -> AcquisitionResult<Self> {
        let channels = channels.into();
        if channels.is_empty() || channels.len() != config.input_mask.count_ones() as usize {
            return Err(AcquisitionError::InvalidRequest(
                "U3Pro16 channel identities must match the enabled physical inputs".into(),
            ));
        }
        let (profile, capture_window_samples) = match config.mode {
            CaptureMode::Finite => (
                CaptureProfile::Buffered,
                DsLogicCapturePlan::new_buffered(&config)
                    .map_err(|error| AcquisitionError::InvalidRequest(error.to_string()))?
                    .actual_samples(),
            ),
            CaptureMode::Streaming => {
                let high = DsLogicCapturePlan::new_streaming(&config, LinkSpeed::High);
                let super_speed = DsLogicCapturePlan::new_streaming(&config, LinkSpeed::Super);
                if let (Err(high), Err(super_speed)) = (high, super_speed) {
                    return Err(AcquisitionError::InvalidRequest(format!(
                        "U3Pro16 stream is unsupported on High Speed ({high}) and SuperSpeed ({super_speed})"
                    )));
                }
                (CaptureProfile::Streaming, config.sample_limit)
            }
        };
        Ok(Self {
            config,
            channels,
            profile,
            capture_window_samples,
            transport_factory,
        })
    }

    /// Returns the delivery behavior selected by this capture request.
    pub const fn data_delivery(&self) -> CaptureDataDelivery {
        match self.profile {
            CaptureProfile::Buffered => CaptureDataDelivery::BufferedUpload,
            CaptureProfile::Streaming => CaptureDataDelivery::DuringAcquisition,
        }
    }

    /// Returns the validated capture window size.
    pub const fn capture_window_samples(&self) -> u64 {
        self.capture_window_samples
    }

    /// Clears hardware triggering for a capture-now request.
    pub fn without_trigger(mut self) -> Self {
        self.config.trigger = Default::default();
        self
    }

    /// Opens and prepares the configured device through the generic acquisition runtime.
    pub fn prepare(
        self,
        context: AcquisitionContext,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        match self.profile {
            CaptureProfile::Buffered => {
                let analyzer = super::implementation::DsLogicU3Pro16::new(
                    self.transport_factory
                        .open()
                        .map_err(|error| AcquisitionError::Transport(error.to_string()))?,
                )
                .map_err(|error| AcquisitionError::Transport(error.to_string()))?;
                BufferedProvider::new(analyzer, self.config, self.channels)?.prepare(context)
            }
            CaptureProfile::Streaming => {
                let analyzer = super::implementation::DsLogicU3Pro16::new(
                    self.transport_factory
                        .open()
                        .map_err(|error| AcquisitionError::Transport(error.to_string()))?,
                )
                .map_err(|error| AcquisitionError::Transport(error.to_string()))?;
                StreamingProvider::new(analyzer, self.config, self.channels)?.prepare(context)
            }
        }
    }
}

impl ConfiguredAcquisition for DsLogicU3Pro16Capture {
    fn data_delivery(&self) -> CaptureDataDelivery {
        self.data_delivery()
    }

    fn capture_window_samples(&self) -> u64 {
        self.capture_window_samples()
    }

    fn prepare(
        self: Box<Self>,
        context: AcquisitionContext,
        mode: CaptureStartMode,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        let capture = if mode == CaptureStartMode::CaptureNow {
            self.without_trigger()
        } else {
            *self
        };
        DsLogicU3Pro16Capture::prepare(capture, context)
    }
}
