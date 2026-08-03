//! Sparse, incrementally recorded binary edge query.

use std::sync::{Arc, RwLock};

use crate::capture::CaptureTransition;
use crate::{EdgeQuery, Result};

/// A queryable binary level whose transitions are appended while a node runs.
///
/// Positions use the nanosecond timeline directly. This is intended for
/// sparse derived control signals, not raw sampled data.
#[derive(Clone, Debug)]
pub struct RecordedEdgeQuery {
    transitions: Arc<RwLock<Vec<CaptureTransition>>>,
}

impl RecordedEdgeQuery {
    /// Creates a sparse query with an initial level at timestamp zero.
    ///
    /// # Parameters
    /// - `initial`: Logic level before any recorded transition.
    pub fn new(initial: bool) -> Self {
        Self {
            transitions: Arc::new(RwLock::new(vec![CaptureTransition {
                sample: 0,
                value: initial,
            }])),
        }
    }

    /// Records a new level at `timestamp_ns`, replacing any later history.
    pub fn record(&self, timestamp_ns: u64, value: bool) {
        let mut transitions = self.transitions.write().unwrap();
        let keep = transitions.partition_point(|edge| edge.sample < timestamp_ns);
        transitions.truncate(keep);
        if transitions.last().is_some_and(|edge| edge.value == value) {
            return;
        }
        transitions.push(CaptureTransition {
            sample: timestamp_ns,
            value,
        });
    }
}

impl EdgeQuery for RecordedEdgeQuery {
    fn sample_period(&self) -> f64 {
        1e-9
    }

    fn samplerate_hz(&self) -> f64 {
        1e9
    }

    fn total_samples(&self) -> u64 {
        u64::MAX
    }

    fn value_at(&self, position: u64) -> Result<bool> {
        let transitions = self.transitions.read().unwrap();
        let index = transitions.partition_point(|edge| edge.sample <= position);
        Ok(transitions[index.saturating_sub(1)].value)
    }

    fn next_edge(&self, position: u64, limit: u64) -> Result<Option<CaptureTransition>> {
        let transitions = self.transitions.read().unwrap();
        let index = transitions.partition_point(|edge| edge.sample <= position);
        Ok(transitions
            .get(index)
            .filter(|edge| edge.sample < limit)
            .cloned())
    }
}

#[cfg(test)]
mod recorded_edge_query_tests {
    use super::*;

    #[test]
    fn recorded_transitions_are_queryable_and_replace_future_history() {
        let query = RecordedEdgeQuery::new(false);
        query.record(20, true);
        query.record(40, false);

        assert!(!query.value_at(19).unwrap());
        assert!(query.value_at(20).unwrap());
        assert_eq!(query.next_edge(20, 100).unwrap().unwrap().sample, 40);

        query.record(30, false);
        assert!(!query.value_at(30).unwrap());
        assert_eq!(query.next_edge(20, 100).unwrap().unwrap().sample, 30);
        assert!(query.next_edge(30, 100).unwrap().is_none());
    }
}
