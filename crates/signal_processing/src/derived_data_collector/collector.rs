use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use web_time::Instant;

use crate::errors::{WorkError, WorkResult};
use crate::node::ProcessNode;
use crate::payload::CollectedLaneIngestor;
use crate::ports::{InputPort, OutputPort, PortSchema};

#[derive(Clone, Default)]
pub struct DerivedDataCollectorMetrics {
    inner: Arc<DerivedDataCollectorMetricsInner>,
}

#[derive(Default)]
struct DerivedDataCollectorMetricsInner {
    drain_ns: AtomicU64,
    append_ns: AtomicU64,
    items: AtomicU64,
    batches: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DerivedDataCollectorMetricsSnapshot {
    pub drain_ns: u64,
    pub append_ns: u64,
    pub items: u64,
    pub batches: u64,
}

impl DerivedDataCollectorMetrics {
    pub fn snapshot(&self) -> DerivedDataCollectorMetricsSnapshot {
        DerivedDataCollectorMetricsSnapshot {
            drain_ns: self.inner.drain_ns.load(Ordering::Relaxed),
            append_ns: self.inner.append_ns.load(Ordering::Relaxed),
            items: self.inner.items.load(Ordering::Relaxed),
            batches: self.inner.batches.load(Ordering::Relaxed),
        }
    }

    fn record_drain(&self, started: Instant, items: usize) {
        self.inner.drain_ns.fetch_add(
            started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );
        if items > 0 {
            self.inner.items.fetch_add(items as u64, Ordering::Relaxed);
            self.inner.batches.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_append(&self, started: Instant) {
        self.inner.append_ns.fetch_add(
            started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );
    }
}

/// Suggested per-lane limit for continuous sources that explicitly select
/// rolling in-memory exact-detail retention. Native indexed word lanes do
/// not use this limit because their complete exact history is disk-backed.
pub const DEFAULT_DERIVED_DATA_MAX_ENTRIES: usize = 1_000_000;

/// Most items one lane drains from its channel per `work()` call. Bounds how
/// long one call holds a lane's storage write lock and, more importantly,
/// stops `DerivedDataCollector` from racing a fast producer to keep its channel
/// perpetually empty — a channel that's allowed to actually fill is what
/// lets its `Block` overflow policy engage and slow the producer down.
pub(crate) const DRAIN_BATCH_SIZE: usize = 65_536;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DerivedDataRetention {
    #[default]
    Unlimited,
    MaxEntries(usize),
}

impl DerivedDataRetention {
    /// Returns the retained entry target when an adapter should trim its
    /// current exact-detail sequence.
    pub fn trim_target(self, len: usize) -> Option<usize> {
        let Self::MaxEntries(max) = self else {
            return None;
        };
        let max = max.max(1);
        (len > max).then_some((max - max / 4).max(1))
    }
}

/// Sink with one typed input per lane. Never blocks *waiting* on any single
/// input — lanes drain round-robin with `try_recv` so a quiet lane cannot
/// stall a busy one — but each lane's channel is drained in bounded batches
/// (`DRAIN_BATCH_SIZE`), not to exhaustion, so a channel that a producer is
/// filling faster than this sink drains it stays full and the producer's
/// own send genuinely blocks (`docs/PIPELINE_DESIGN.md`, flow control) — real
/// backpressure, not a silent drop once storage fills up.
pub struct DerivedDataCollector {
    name: String,
    lanes: Vec<Box<dyn CollectedLaneIngestor>>,
    retention: DerivedDataRetention,
    metrics: Option<DerivedDataCollectorMetrics>,
}

impl DerivedDataCollector {
    pub fn new() -> Self {
        Self {
            name: "derived-data-collector".to_owned(),
            lanes: Vec::new(),
            retention: DerivedDataRetention::Unlimited,
            metrics: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_retention(mut self, retention: DerivedDataRetention) -> Self {
        self.retention = retention;
        self
    }

    pub fn with_metrics(mut self, metrics: DerivedDataCollectorMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Adds an adapter-owned lane; input port order follows insertion order.
    pub fn with_ingestor(mut self, ingestor: Box<dyn CollectedLaneIngestor>) -> Self {
        self.lanes.push(ingestor);
        self
    }
}

impl Default for DerivedDataCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessNode for DerivedDataCollector {
    fn name(&self) -> &str {
        &self.name
    }

    fn should_stop(&self) -> bool {
        !self.lanes.is_empty() && self.lanes.iter().all(|lane| lane.is_finished())
    }

    fn input_scheduling(&self) -> crate::node::InputScheduling {
        crate::node::InputScheduling::Any
    }

    fn num_inputs(&self) -> usize {
        self.lanes.len()
    }

    fn num_outputs(&self) -> usize {
        0
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        self.lanes
            .iter()
            .enumerate()
            .map(|(index, lane)| lane.input_schema(index))
            .collect()
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        Vec::new()
    }

    fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
        let mut progress = 0;
        for (index, lane) in self.lanes.iter_mut().enumerate() {
            if lane.is_finished() {
                continue;
            }
            let input = inputs
                .get(index)
                .ok_or_else(|| WorkError::NodeError(format!("missing collector input {index}")))?;
            let started = self.metrics.as_ref().map(|_| Instant::now());
            let drained = lane.drain(input, self.retention)?;
            progress += drained;
            if let (Some(metrics), Some(started)) = (&self.metrics, started) {
                metrics.record_drain(started, drained);
                if drained > 0 {
                    metrics.record_append(started);
                }
            }
        }
        if progress == 0 {
            if self.lanes.iter().all(|lane| lane.is_finished()) {
                return Err(WorkError::Shutdown);
            }
            crate::idle_backoff();
        }
        Ok(progress)
    }
}
