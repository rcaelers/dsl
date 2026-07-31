use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tracing::warn;

use signal_processing::{
    EdgeQuery, InputPort, SamplingPoint, SamplingPointProvider, SamplingPointStore,
};

use super::types::StrobeMode;
use crate::types::CsPolarity;

const EDGE_BATCH_SIZE: usize = 4_096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SampleRange {
    start: u64,
    end: u64,
}

#[derive(Debug, Default)]
struct ParallelSamplingProgressInner {
    watermark: AtomicU64,
    revision: AtomicU64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ParallelSamplingProgress {
    inner: Arc<ParallelSamplingProgressInner>,
}

impl ParallelSamplingProgress {
    pub(crate) fn advance(&self, watermark: u64) {
        if self.inner.watermark.fetch_max(watermark, Ordering::AcqRel) < watermark {
            self.inner.revision.fetch_add(1, Ordering::Release);
        }
    }

    fn watermark(&self) -> u64 {
        self.inner.watermark.load(Ordering::Acquire)
    }

    fn revision(&self) -> u64 {
        self.inner.revision.load(Ordering::Acquire)
    }
}

enum EnableSource {
    Always,
    Query(Arc<dyn EdgeQuery>),
}

#[derive(Clone)]
struct CachedQuery {
    start_ns: u64,
    end_ns: u64,
    minimum_spacing_ns: u64,
    revision: u64,
    points: Option<Vec<SamplingPoint>>,
}

struct ParallelSamplingProvider {
    strobe: Arc<dyn EdgeQuery>,
    data: Vec<Arc<dyn EdgeQuery>>,
    cs: Option<Arc<dyn EdgeQuery>>,
    enable: EnableSource,
    mode: StrobeMode,
    cs_polarity: CsPolarity,
    timestamp_step: u64,
    progress: ParallelSamplingProgress,
    cache: Mutex<Option<CachedQuery>>,
}

impl fmt::Debug for ParallelSamplingProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParallelSamplingProvider")
            .field("data_channels", &self.data.len())
            .field("mode", &self.mode)
            .field("cs_polarity", &self.cs_polarity)
            .field("timestamp_step", &self.timestamp_step)
            .finish_non_exhaustive()
    }
}

impl SamplingPointProvider for ParallelSamplingProvider {
    fn points_in_range_with_minimum_spacing(
        &self,
        start_ns: u64,
        end_ns: u64,
        minimum_spacing_ns: u64,
    ) -> Option<Vec<SamplingPoint>> {
        let revision = self.progress.revision();
        if let Some(cached) = self.cache.lock().unwrap().as_ref()
            && cached.start_ns == start_ns
            && cached.end_ns == end_ns
            && cached.minimum_spacing_ns == minimum_spacing_ns
            && cached.revision == revision
        {
            return cached.points.clone();
        }

        let points = self
            .query(start_ns, end_ns, minimum_spacing_ns)
            .unwrap_or_else(|error| {
                warn!(%error, "could not query parallel-decoder sampling points");
                Some(Vec::new())
            });
        *self.cache.lock().unwrap() = Some(CachedQuery {
            start_ns,
            end_ns,
            minimum_spacing_ns,
            revision,
            points: points.clone(),
        });
        points
    }
}

impl ParallelSamplingProvider {
    fn query(
        &self,
        start_ns: u64,
        end_ns: u64,
        minimum_spacing_ns: u64,
    ) -> signal_processing::Result<Option<Vec<SamplingPoint>>> {
        if start_ns > end_ns || self.progress.watermark() == 0 {
            return Ok(Some(Vec::new()));
        }
        let start = start_ns.div_ceil(self.timestamp_step);
        let requested_end = end_ns.saturating_div(self.timestamp_step).saturating_add(1);
        let end = requested_end
            .min(self.progress.watermark())
            .min(self.strobe.total_samples());
        if start >= end {
            return Ok(Some(Vec::new()));
        }

        let enable_ranges = match &self.enable {
            EnableSource::Always => vec![SampleRange { start, end }],
            EnableSource::Query(query) => query_level_ranges(query, true, start, end)?,
        };
        let eligible_ranges = match (&self.cs, self.cs_polarity) {
            (Some(query), CsPolarity::ActiveLow) => intersect_ranges(
                &enable_ranges,
                &query_level_ranges(query, false, start, end)?,
            ),
            (Some(query), CsPolarity::ActiveHigh) => intersect_ranges(
                &enable_ranges,
                &query_level_ranges(query, true, start, end)?,
            ),
            _ => enable_ranges,
        };

        let mut positions = Vec::new();
        let mut clock_values = Vec::new();
        let mut edges = Vec::new();
        let mut previous_time = None;
        for range in eligible_ranges {
            let mut cursor = range.start.saturating_sub(1);
            loop {
                self.strobe
                    .next_edges(cursor, range.end, EDGE_BATCH_SIZE, &mut edges)?;
                edges.retain(|edge| edge.sample >= range.start && edge.sample < range.end);
                for edge in &edges {
                    let accepted = match self.mode {
                        StrobeMode::RisingEdge => edge.value,
                        StrobeMode::FallingEdge => !edge.value,
                        StrobeMode::AnyEdge => true,
                        StrobeMode::HighLevel | StrobeMode::LowLevel => false,
                    };
                    if !accepted {
                        continue;
                    }
                    let time_ns = edge.sample.saturating_mul(self.timestamp_step);
                    if previous_time.is_some_and(|previous| {
                        time_ns.saturating_sub(previous) < minimum_spacing_ns
                    }) {
                        return Ok(None);
                    }
                    previous_time = Some(time_ns);
                    positions.push(edge.sample);
                    clock_values.push(edge.value);
                }
                let Some(last) = edges.last() else {
                    break;
                };
                cursor = last.sample;
                if edges.len() < EDGE_BATCH_SIZE {
                    break;
                }
            }
        }

        let mut values = Vec::with_capacity(self.data.len());
        for query in &self.data {
            let mut channel_values = Vec::new();
            query.values_at(&positions, &mut channel_values)?;
            values.push(channel_values);
        }
        Ok(Some(
            positions
                .into_iter()
                .enumerate()
                .map(|(index, position)| {
                    SamplingPoint::new(
                        position.saturating_mul(self.timestamp_step),
                        clock_values[index],
                        values
                            .iter()
                            .map(|channel| channel[index])
                            .collect::<Vec<_>>(),
                    )
                })
                .collect(),
        ))
    }
}

pub(crate) fn install_sampling_provider(
    store: &SamplingPointStore,
    inputs: &[InputPort],
    num_data_bits: usize,
    mode: StrobeMode,
    cs_polarity: CsPolarity,
    progress: ParallelSamplingProgress,
) {
    if store.has_provider() {
        return;
    }
    let Some(strobe) = inputs.first().and_then(InputPort::edge_query_capability) else {
        return;
    };
    let data = (0..num_data_bits)
        .map(|index| {
            inputs
                .get(1 + index)
                .and_then(InputPort::edge_query_capability)
        })
        .collect::<Option<Vec<_>>>();
    let Some(data) = data else {
        return;
    };
    let cs_index = 1 + num_data_bits;
    let cs = inputs
        .get(cs_index)
        .and_then(InputPort::edge_query_capability);
    if cs_polarity != CsPolarity::Disabled && cs.is_none() {
        return;
    }
    let enable = match inputs.get(cs_index + 1) {
        Some(input) => match input.edge_query_capability() {
            Some(query) => EnableSource::Query(query),
            None if input.is_connected() => return,
            None => EnableSource::Always,
        },
        None => EnableSource::Always,
    };
    let timestamp_step = (1_000_000_000.0 / strobe.samplerate_hz()) as u64;
    store.set_provider(Arc::new(ParallelSamplingProvider {
        strobe,
        data,
        cs,
        enable,
        mode,
        cs_polarity,
        timestamp_step: timestamp_step.max(1),
        progress,
        cache: Mutex::new(None),
    }));
}

fn query_level_ranges(
    query: &Arc<dyn EdgeQuery>,
    active: bool,
    start: u64,
    end: u64,
) -> signal_processing::Result<Vec<SampleRange>> {
    let mut value = query.value_at(start)?;
    let mut cursor = start;
    let mut ranges = Vec::new();
    while cursor < end {
        let transition = query
            .next_edge(cursor, end)?
            .filter(|transition| transition.sample < end);
        let range_end = transition
            .as_ref()
            .map_or(end, |transition| transition.sample);
        if value == active && cursor < range_end {
            ranges.push(SampleRange {
                start: cursor,
                end: range_end,
            });
        }
        let Some(transition) = transition else {
            break;
        };
        cursor = transition.sample;
        value = transition.value;
    }
    Ok(ranges)
}

fn intersect_ranges(left: &[SampleRange], right: &[SampleRange]) -> Vec<SampleRange> {
    let mut intersections = Vec::new();
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() && right_index < right.len() {
        let start = left[left_index].start.max(right[right_index].start);
        let end = left[left_index].end.min(right[right_index].end);
        if start < end {
            intersections.push(SampleRange { start, end });
        }
        if left[left_index].end <= right[right_index].end {
            left_index += 1;
        } else {
            right_index += 1;
        }
    }
    intersections
}

#[cfg(test)]
mod sampling_provider_tests {
    use super::*;

    struct FakeQuery {
        bits: Vec<bool>,
    }

    impl EdgeQuery for FakeQuery {
        fn sample_period(&self) -> f64 {
            1e-9
        }

        fn samplerate_hz(&self) -> f64 {
            1e9
        }

        fn total_samples(&self) -> u64 {
            self.bits.len() as u64
        }

        fn value_at(&self, position: u64) -> signal_processing::Result<bool> {
            Ok(self.bits[position as usize])
        }

        fn next_edge(
            &self,
            position: u64,
            limit: u64,
        ) -> signal_processing::Result<Option<signal_processing::capture::CaptureTransition>>
        {
            let mut value = self.bits[position as usize];
            for sample in position.saturating_add(1)..limit.min(self.total_samples()) {
                let next = self.bits[sample as usize];
                if next != value {
                    return Ok(Some(signal_processing::capture::CaptureTransition {
                        sample,
                        value: next,
                    }));
                }
                value = next;
            }
            Ok(None)
        }
    }

    fn query_input(bits: &[bool]) -> InputPort {
        InputPort::disconnected().with_edge_query_capability(Some(Arc::new(FakeQuery {
            bits: bits.to_vec(),
        })))
    }

    #[test]
    fn range_intersection_preserves_only_shared_intervals() {
        assert_eq!(
            intersect_ranges(
                &[
                    SampleRange { start: 1, end: 5 },
                    SampleRange { start: 8, end: 12 },
                ],
                &[
                    SampleRange { start: 3, end: 9 },
                    SampleRange { start: 10, end: 11 },
                ],
            ),
            [
                SampleRange { start: 3, end: 5 },
                SampleRange { start: 8, end: 9 },
                SampleRange { start: 10, end: 11 },
            ]
        );
    }

    #[test]
    fn provider_queries_processed_parallel_edges_without_recording_them() {
        let store = SamplingPointStore::disabled();
        let progress = ParallelSamplingProgress::default();
        progress.advance(8);
        let inputs = [
            query_input(&[false, false, true, true, false, false, true, true]),
            query_input(&[false, false, true, true, false, false, false, false]),
            InputPort::disconnected(),
            InputPort::disconnected(),
        ];

        install_sampling_provider(
            &store,
            &inputs,
            1,
            StrobeMode::AnyEdge,
            CsPolarity::Disabled,
            progress,
        );

        assert!(store.has_provider());
        assert!(!store.is_recording_enabled());
        assert_eq!(
            store.points_in_range(0, 7),
            [
                SamplingPoint::new(2, true, vec![true]),
                SamplingPoint::new(4, false, vec![false]),
                SamplingPoint::new(6, true, vec![false]),
            ]
        );
        assert!(
            store
                .points_in_range_with_minimum_spacing(0, 7, 3)
                .is_none()
        );
    }
}
