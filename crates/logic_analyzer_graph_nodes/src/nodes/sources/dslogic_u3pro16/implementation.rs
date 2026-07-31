use std::time::Duration;

use signal_processing::{
    CaptureFraction, CapturePolicy, CompletionPolicy, RecordingStart, RetentionPolicy,
    TriggerPlacement, TriggerTimeout, TriggerTimeoutAction,
};

use super::capture_configuration::capture_config;
use super::definition::U3Pro16State;

fn retention_policy(state: &U3Pro16State) -> RetentionPolicy {
    match state.retention.selected() {
        "Recent duration" => RetentionPolicy::RecentDuration(Duration::from_millis(
            u64::try_from(state.retention_duration_ms.value.max(1)).unwrap_or(1),
        )),
        "Recent bytes" => RetentionPolicy::RecentBytes(
            u64::try_from(state.retention_megabytes.value.max(1))
                .unwrap_or(1)
                .saturating_mul(1024 * 1024),
        ),
        _ => RetentionPolicy::Everything,
    }
}

pub(crate) fn requested_capture_policy(state: &U3Pro16State) -> Result<CapturePolicy, String> {
    let config = capture_config(state)?;
    let start = if state.recording_start.selected() == "Trigger" {
        RecordingStart::Trigger
    } else {
        RecordingStart::Immediate
    };
    if start == RecordingStart::Trigger && config.trigger.stages.is_empty() {
        return Err("triggered recording requires at least one enabled trigger condition".into());
    }
    let before_samples = if start == RecordingStart::Trigger {
        config
            .sample_limit
            .saturating_mul(u64::from(config.trigger_percent))
            / 100
    } else {
        0
    };
    let trigger_timeout = match state.trigger_timeout_action.selected() {
        "Continue waiting" => Some(TriggerTimeout {
            after: Duration::from_millis(
                u64::try_from(state.trigger_timeout_ms.value.max(1)).unwrap_or(1),
            ),
            action: TriggerTimeoutAction::ContinueWaiting,
        }),
        "Stop" => Some(TriggerTimeout {
            after: Duration::from_millis(
                u64::try_from(state.trigger_timeout_ms.value.max(1)).unwrap_or(1),
            ),
            action: TriggerTimeoutAction::Stop,
        }),
        _ => None,
    };
    Ok(CapturePolicy {
        start,
        trigger_placement: (start == RecordingStart::Trigger).then(|| {
            TriggerPlacement::Fraction(
                CaptureFraction::from_percent(config.trigger_percent)
                    .expect("clamped trigger percentage is valid"),
            )
        }),
        retention_before_origin: RetentionPolicy::Everything,
        retention_after_origin: retention_policy(state),
        completion: CompletionPolicy::SamplesAfterOrigin(
            config.sample_limit.saturating_sub(before_samples).max(1),
        ),
        trigger_timeout,
    })
}

#[cfg(test)]
mod tests {
    use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::{
        CaptureMode, ClockEdge, ClockSource, LogicEncodingRequest, TriggerCondition,
    };
    use signal_processing::SimpleTriggerCondition;

    use super::super::definition::CaptureDurationValue;
    use super::{U3Pro16State, capture_config};

    #[test]
    fn buffered_state_lowers_channels_depth_trigger_timebase_and_encoding() {
        let mut state = U3Pro16State::default();
        state.mode.select("Buffer");
        state.sample_rate.select("100 MHz");
        state.duration = CaptureDurationValue::from_milliseconds(10);
        state.channels.enabled.fill(false);
        state.channels.enabled[0] = true;
        state.channels.enabled[2] = true;
        state
            .set_trigger_condition(2, SimpleTriggerCondition::Falling)
            .unwrap();
        state.ext_clock.value = true;
        state.clock_edge.select("Falling");
        state.rle.value = true;
        state.filter.value = true;
        state.threshold.value = 1.25;

        let config = capture_config(&state).unwrap();

        assert_eq!(config.mode, CaptureMode::Finite);
        assert_eq!(config.sample_rate_hz, 100_000_000);
        assert_eq!(config.input_mask, 0b0101);
        assert_eq!(config.sample_limit, 1_000_000);
        assert_eq!(config.trigger_percent, 50);
        assert_eq!(config.threshold_volts, Some(1.25));
        assert_eq!(config.encoding, LogicEncodingRequest::RunLength);
        assert_eq!(
            config.clock,
            ClockSource::External {
                edge: ClockEdge::Falling
            }
        );
        assert!(config.input_filter);
        assert_eq!(config.trigger.stages.len(), 1);
        assert_eq!(
            config.trigger.stages[0].plane0[2],
            TriggerCondition::Falling
        );
        assert_eq!(config.trigger.stages[0].plane0[1], TriggerCondition::Ignore);
    }

    #[test]
    fn microsecond_capture_duration_lowers_to_samples() {
        let mut state = U3Pro16State::default();
        state.sample_rate.select("100 MHz");
        state.duration = CaptureDurationValue::from_nanoseconds(10_000);

        let config = capture_config(&state).unwrap();

        assert_eq!(config.sample_limit, 1_000);
    }

    #[test]
    fn streaming_capture_is_capped_at_the_dsview_sample_depth() {
        let mut state = U3Pro16State::default();
        state.sample_rate.select("1 MHz");
        state.duration = CaptureDurationValue::from_nanoseconds(u64::MAX);

        let config = capture_config(&state).unwrap();

        assert_eq!(config.sample_limit, 1_u64 << 34);
    }

    #[test]
    fn capture_config_rejects_too_many_channels_for_stream_rate() {
        let mut state = U3Pro16State::default();
        state.sample_rate.select("1 GHz");

        let error = capture_config(&state).unwrap_err();

        assert!(error.contains("Too many channels"));
        assert!(error.contains("Ch 0–2"));
    }
}
