use signal_capture_session::logic_analyzer::{
    CaptureMode, ClockEdge, ClockSource, LogicCaptureConfig, LogicEncodingRequest, LogicTrigger,
};

use super::definition::{U3Pro16State, capture_duration_limit_ns, channel_rate_validation_error};
use super::trigger_lowering;

fn selected_sample_rate_hz(state: &U3Pro16State) -> Result<u64, String> {
    state
        .sample_rate
        .selected()
        .strip_suffix(" GHz")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value * 1_000_000_000)
        .or_else(|| {
            state
                .sample_rate
                .selected()
                .strip_suffix(" MHz")
                .and_then(|value| value.parse::<u64>().ok())
                .map(|value| value * 1_000_000)
        })
        .ok_or_else(|| "invalid U3Pro16 sample rate".to_owned())
}

fn physical_input_mask(state: &U3Pro16State) -> u64 {
    state
        .channels
        .enabled
        .iter()
        .enumerate()
        .fold(0_u64, |mask, (index, enabled)| {
            if *enabled {
                mask | (1_u64 << index)
            } else {
                mask
            }
        })
}

pub(crate) fn capture_config(state: &U3Pro16State) -> Result<LogicCaptureConfig, String> {
    if let Some(error) = channel_rate_validation_error(state) {
        return Err(error);
    }
    let sample_rate_hz = selected_sample_rate_hz(state)?;
    let enabled_channels = state.channels.enabled_count();
    let duration_ns = state.duration.nanoseconds().min(capture_duration_limit_ns(
        state.mode.selected(),
        sample_rate_hz,
        enabled_channels,
    ));
    let sample_limit = (u128::from(sample_rate_hz) * u128::from(duration_ns))
        .div_ceil(1_000_000_000)
        .min(u128::from(u64::MAX)) as u64;
    Ok(LogicCaptureConfig {
        mode: if state.mode.selected() == "Stream" {
            CaptureMode::Streaming
        } else {
            CaptureMode::Finite
        },
        sample_rate_hz,
        input_mask: physical_input_mask(state),
        sample_limit,
        trigger_percent: u8::try_from(state.trigger_position_percent.value.clamp(0, 100))
            .unwrap_or(50),
        threshold_volts: Some(state.threshold.value),
        trigger: if state.recording_start.selected() == "Trigger" {
            trigger_lowering::lower_program(state)?
        } else {
            LogicTrigger::default()
        },
        encoding: if state.rle.value {
            LogicEncodingRequest::RunLength
        } else {
            LogicEncodingRequest::Raw
        },
        clock: if state.ext_clock.value {
            ClockSource::External {
                edge: if state.clock_edge.selected() == "Falling" {
                    ClockEdge::Falling
                } else {
                    ClockEdge::Rising
                },
            }
        } else {
            ClockSource::Internal
        },
        input_filter: state.filter.value,
    })
}
