//! Lazy random-access query for a composed logic-gate output.

use std::sync::Arc;

use signal_capture::{CaptureTransition, EdgeQuery, Result};

use super::implementation::GateOp;

pub(crate) struct LogicGateEdgeQuery {
    op: GateOp,
    inputs: Vec<Arc<dyn EdgeQuery>>,
    samplerate_hz: f64,
    total_samples: u64,
}

impl LogicGateEdgeQuery {
    pub(crate) fn new(op: GateOp, inputs: Vec<Arc<dyn EdgeQuery>>) -> Option<Self> {
        let first = inputs.first()?;
        let samplerate_hz = first.samplerate_hz();
        let total_samples = inputs
            .iter()
            .map(|input| scale_floor(input.total_samples(), samplerate_hz, input.samplerate_hz()))
            .min()
            .unwrap_or(0);
        Some(Self {
            op,
            inputs,
            samplerate_hz,
            total_samples,
        })
    }

    fn input_levels(&self, position: u64) -> Result<Vec<bool>> {
        self.inputs
            .iter()
            .map(|input| {
                input.value_at(scale_floor(
                    position,
                    input.samplerate_hz(),
                    self.samplerate_hz,
                ))
            })
            .collect()
    }

    fn combined_value(&self, position: u64) -> Result<bool> {
        Ok(self.op.combine(&self.input_levels(position)?))
    }

    fn inputs_that_can_change_output(&self, levels: &[bool]) -> Vec<usize> {
        let all = || (0..levels.len()).collect();
        match self.op {
            GateOp::And | GateOp::Nand if !levels.iter().all(|level| *level) => levels
                .iter()
                .enumerate()
                .filter_map(|(index, level)| (!level).then_some(index))
                .collect(),
            GateOp::Or | GateOp::Nor if levels.iter().any(|level| *level) => levels
                .iter()
                .enumerate()
                .filter_map(|(index, level)| level.then_some(index))
                .collect(),
            _ => all(),
        }
    }

    fn next_input_position(
        &self,
        input: &Arc<dyn EdgeQuery>,
        position: u64,
        limit: u64,
    ) -> Result<Option<u64>> {
        let mut input_position = scale_floor(position, input.samplerate_hz(), self.samplerate_hz);
        let input_limit =
            scale_ceil(limit, input.samplerate_hz(), self.samplerate_hz).min(input.total_samples());
        loop {
            let Some(edge) = input.next_edge(input_position, input_limit)? else {
                return Ok(None);
            };
            input_position = edge.sample;
            let output_position =
                scale_ceil(edge.sample, self.samplerate_hz, input.samplerate_hz());
            if output_position > position && output_position < limit {
                return Ok(Some(output_position));
            }
        }
    }
}

impl EdgeQuery for LogicGateEdgeQuery {
    fn sample_period(&self) -> f64 {
        1.0 / self.samplerate_hz
    }

    fn samplerate_hz(&self) -> f64 {
        self.samplerate_hz
    }

    fn total_samples(&self) -> u64 {
        self.total_samples
    }

    fn value_at(&self, position: u64) -> Result<bool> {
        self.combined_value(position)
    }

    fn next_edge(&self, position: u64, limit: u64) -> Result<Option<CaptureTransition>> {
        let mut cursor = position;
        while cursor < limit {
            let levels = self.input_levels(cursor)?;
            let value = self.op.combine(&levels);
            let next = self
                .inputs_that_can_change_output(&levels)
                .into_iter()
                .map(|index| self.next_input_position(&self.inputs[index], cursor, limit))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .min();
            let Some(next) = next else {
                return Ok(None);
            };
            let next_value = self.combined_value(next)?;
            if next_value != value {
                return Ok(Some(CaptureTransition {
                    sample: next,
                    value: next_value,
                }));
            }
            cursor = next;
        }
        Ok(None)
    }
}

fn scale_floor(position: u64, target_rate: f64, source_rate: f64) -> u64 {
    saturating_u64(position as f64 * target_rate / source_rate, f64::floor)
}

fn scale_ceil(position: u64, target_rate: f64, source_rate: f64) -> u64 {
    saturating_u64(position as f64 * target_rate / source_rate, f64::ceil)
}

fn saturating_u64(value: f64, round: impl FnOnce(f64) -> f64) -> u64 {
    let value = round(value);
    if !value.is_finite() || value >= u64::MAX as f64 {
        u64::MAX
    } else if value <= 0.0 {
        0
    } else {
        value as u64
    }
}

#[cfg(test)]
mod edge_query_tests {
    use signal_capture::RecordedEdgeQuery;

    use super::*;

    struct FixedRateQuery {
        samplerate_hz: f64,
        total_samples: u64,
        transitions: Vec<CaptureTransition>,
    }

    impl EdgeQuery for FixedRateQuery {
        fn sample_period(&self) -> f64 {
            1.0 / self.samplerate_hz
        }

        fn samplerate_hz(&self) -> f64 {
            self.samplerate_hz
        }

        fn total_samples(&self) -> u64 {
            self.total_samples
        }

        fn value_at(&self, position: u64) -> Result<bool> {
            let index = self
                .transitions
                .partition_point(|edge| edge.sample <= position);
            Ok(self.transitions[index.saturating_sub(1)].value)
        }

        fn next_edge(&self, position: u64, limit: u64) -> Result<Option<CaptureTransition>> {
            let index = self
                .transitions
                .partition_point(|edge| edge.sample <= position);
            Ok(self
                .transitions
                .get(index)
                .filter(|edge| edge.sample < limit)
                .cloned())
        }
    }

    #[test]
    fn composed_query_combines_different_time_bases_lazily() {
        let raw = FixedRateQuery {
            samplerate_hz: 50e6,
            total_samples: 10,
            transitions: vec![
                CaptureTransition {
                    sample: 0,
                    value: false,
                },
                CaptureTransition {
                    sample: 1,
                    value: true,
                },
                CaptureTransition {
                    sample: 4,
                    value: false,
                },
            ],
        };
        let control = RecordedEdgeQuery::new(false);
        control.record(40, true);
        control.record(60, false);
        let query =
            LogicGateEdgeQuery::new(GateOp::And, vec![Arc::new(raw), Arc::new(control)]).unwrap();

        assert!(!query.value_at(1).unwrap());
        assert!(query.value_at(2).unwrap());
        assert!(!query.value_at(3).unwrap());
        assert_eq!(query.next_edge(0, 10).unwrap().unwrap().sample, 2);
        assert_eq!(query.next_edge(2, 10).unwrap().unwrap().sample, 3);
    }
}
