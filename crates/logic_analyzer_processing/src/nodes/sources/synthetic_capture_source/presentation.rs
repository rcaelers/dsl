use super::implementation::SyntheticCaptureSource;
use crate::{CaptureSourcePresentation, CaptureSourceSignal};

pub(crate) fn synthetic_presentation(
    channel_names: impl IntoIterator<Item = String>,
    excluded_channels: &[usize],
) -> CaptureSourcePresentation {
    let channel_names = channel_names.into_iter().collect::<Vec<_>>();
    let channels = SyntheticCaptureSource::preview_channels_with_count(channel_names.len());
    let signals = channel_names
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !excluded_channels.contains(index))
        .map(|(index, name)| {
            let samples = &channels[index];
            CaptureSourceSignal::new(
                index,
                name,
                samples.first().is_some_and(|sample| sample.value),
                samples
                    .iter()
                    .skip(1)
                    .map(|sample| (sample.start_time_ns as f64 / 1_000.0, sample.value))
                    .collect(),
            )
        })
        .collect::<Vec<_>>();
    let duration_us = signals
        .iter()
        .filter_map(|signal| signal.transitions().last().map(|(time, _)| *time))
        .fold(1.0_f64, f64::max);
    CaptureSourcePresentation::InMemory {
        signals,
        duration_us,
    }
}
