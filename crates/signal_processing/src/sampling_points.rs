use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

/// One sampling decision produced by a clocked processing node.
///
/// Values follow the order of the node's declared sampled inputs. The
/// record deliberately carries no protocol or viewer knowledge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SamplingPoint {
    pub time_ns: u64,
    pub clock_high: bool,
    pub values: Vec<bool>,
}

impl SamplingPoint {
    pub fn new(time_ns: u64, clock_high: bool, values: impl Into<Vec<bool>>) -> Self {
        Self {
            time_ns,
            clock_high,
            values: values.into(),
        }
    }
}

/// Random-access source of sampling decisions for a processed time range.
///
/// Concrete processing nodes implement their own sampling semantics. The
/// generic store and its presentation consumers only request already-accepted
/// decisions for the visible range.
pub trait SamplingPointProvider: std::fmt::Debug + Send + Sync {
    /// Returns every accepted point in the range, or `None` when the complete
    /// range is denser than `minimum_spacing_ns` and should remain hidden.
    fn points_in_range_with_minimum_spacing(
        &self,
        start_ns: u64,
        end_ns: u64,
        minimum_spacing_ns: u64,
    ) -> Option<Vec<SamplingPoint>>;
}

/// Run-owned, thread-safe cache of sampling decisions produced by a node.
///
/// Writers append chronological batches while viewers take inexpensive
/// snapshots of only the visible time range. Recording an earlier time
/// replaces stale data from that point onward, which also supports a live
/// node restarting against the same retained presentation handle.
#[derive(Debug)]
struct SamplingPointStoreInner {
    recording_enabled: AtomicBool,
    points: RwLock<Vec<SamplingPoint>>,
    provider: RwLock<Option<Arc<dyn SamplingPointProvider>>>,
}

#[derive(Clone, Debug)]
pub struct SamplingPointStore {
    inner: Arc<SamplingPointStoreInner>,
}

impl Default for SamplingPointStore {
    fn default() -> Self {
        Self::with_recording_enabled(true)
    }
}

impl SamplingPointStore {
    /// Creates an empty store that ignores records until explicitly enabled.
    pub fn disabled() -> Self {
        Self::with_recording_enabled(false)
    }

    fn with_recording_enabled(recording_enabled: bool) -> Self {
        Self {
            inner: Arc::new(SamplingPointStoreInner {
                recording_enabled: AtomicBool::new(recording_enabled),
                points: RwLock::new(Vec::new()),
                provider: RwLock::new(None),
            }),
        }
    }

    pub fn set_recording_enabled(&self, enabled: bool) {
        let enabled = enabled && !self.has_provider();
        self.inner
            .recording_enabled
            .store(enabled, Ordering::Release);
    }

    pub fn is_recording_enabled(&self) -> bool {
        self.inner.recording_enabled.load(Ordering::Acquire)
    }

    pub fn set_provider(&self, provider: Arc<dyn SamplingPointProvider>) {
        *self.inner.provider.write().unwrap() = Some(provider);
        self.inner.recording_enabled.store(false, Ordering::Release);
    }

    pub fn has_provider(&self) -> bool {
        self.inner.provider.read().unwrap().is_some()
    }

    pub fn record(&self, point: SamplingPoint) {
        self.record_batch([point]);
    }

    pub fn record_batch(&self, points: impl IntoIterator<Item = SamplingPoint>) {
        if !self.is_recording_enabled() {
            return;
        }
        let mut points = points.into_iter().peekable();
        let Some(first) = points.peek() else {
            return;
        };

        let mut stored = self.inner.points.write().unwrap();
        let keep = stored.partition_point(|point| point.time_ns < first.time_ns);
        if keep < stored.len() {
            stored.truncate(keep);
        }

        for point in points {
            if stored
                .last()
                .is_some_and(|previous| previous.time_ns > point.time_ns)
            {
                let keep = stored.partition_point(|stored| stored.time_ns < point.time_ns);
                stored.truncate(keep);
            }
            if stored
                .last()
                .is_some_and(|previous| previous.time_ns == point.time_ns)
            {
                stored.pop();
            }
            stored.push(point);
        }
    }

    pub fn points_in_range(&self, start_ns: u64, end_ns: u64) -> Vec<SamplingPoint> {
        self.points_in_range_with_minimum_spacing(start_ns, end_ns, 0)
            .unwrap_or_default()
    }

    /// Returns the complete range only when every adjacent point meets the
    /// requested spacing. This lets presentation consumers enforce an
    /// all-or-nothing density policy without first cloning a dense range.
    pub fn points_in_range_with_minimum_spacing(
        &self,
        start_ns: u64,
        end_ns: u64,
        minimum_spacing_ns: u64,
    ) -> Option<Vec<SamplingPoint>> {
        if start_ns > end_ns {
            return Some(Vec::new());
        }
        let provider = self.inner.provider.read().unwrap().clone();
        if let Some(provider) = provider {
            return provider.points_in_range_with_minimum_spacing(
                start_ns,
                end_ns,
                minimum_spacing_ns,
            );
        }
        let stored = self.inner.points.read().unwrap();
        let start = stored.partition_point(|point| point.time_ns < start_ns);
        let end = stored.partition_point(|point| point.time_ns <= end_ns);
        let visible = &stored[start..end];
        if visible
            .windows(2)
            .any(|pair| pair[1].time_ns.saturating_sub(pair[0].time_ns) < minimum_spacing_ns)
        {
            return None;
        }
        Some(visible.to_vec())
    }

    pub fn clear(&self) {
        self.inner.points.write().unwrap().clear();
    }

    pub fn is_empty(&self) -> bool {
        self.inner.points.read().unwrap().is_empty()
    }
}

#[cfg(test)]
mod sampling_point_store_tests {
    use super::*;

    #[derive(Debug)]
    struct FixedProvider;

    impl SamplingPointProvider for FixedProvider {
        fn points_in_range_with_minimum_spacing(
            &self,
            start_ns: u64,
            end_ns: u64,
            minimum_spacing_ns: u64,
        ) -> Option<Vec<SamplingPoint>> {
            let points = [
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(20, false, vec![true]),
            ]
            .into_iter()
            .filter(|point| (start_ns..=end_ns).contains(&point.time_ns))
            .collect::<Vec<_>>();
            if points
                .windows(2)
                .any(|pair| pair[1].time_ns.saturating_sub(pair[0].time_ns) < minimum_spacing_ns)
            {
                None
            } else {
                Some(points)
            }
        }
    }

    #[test]
    fn visible_range_is_inclusive_and_ordered() {
        let store = SamplingPointStore::default();
        store.record_batch([
            SamplingPoint::new(10, true, vec![false]),
            SamplingPoint::new(20, false, vec![true]),
            SamplingPoint::new(30, true, vec![false]),
        ]);

        assert_eq!(
            store.points_in_range(20, 30),
            vec![
                SamplingPoint::new(20, false, vec![true]),
                SamplingPoint::new(30, true, vec![false]),
            ]
        );
    }

    #[test]
    fn recording_from_an_earlier_time_replaces_stale_points() {
        let store = SamplingPointStore::default();
        store.record_batch([
            SamplingPoint::new(10, true, vec![false]),
            SamplingPoint::new(30, true, vec![false]),
        ]);
        store.record_batch([
            SamplingPoint::new(20, false, vec![true]),
            SamplingPoint::new(40, false, vec![true]),
        ]);

        assert_eq!(
            store.points_in_range(0, u64::MAX),
            vec![
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(20, false, vec![true]),
                SamplingPoint::new(40, false, vec![true]),
            ]
        );
    }

    #[test]
    fn minimum_spacing_rejects_the_complete_dense_range() {
        let store = SamplingPointStore::default();
        store.record_batch([
            SamplingPoint::new(10, true, vec![false]),
            SamplingPoint::new(14, false, vec![true]),
            SamplingPoint::new(30, true, vec![false]),
        ]);

        assert!(
            store
                .points_in_range_with_minimum_spacing(0, 40, 5)
                .is_none()
        );
        assert_eq!(
            store
                .points_in_range_with_minimum_spacing(0, 40, 4)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn disabled_store_ignores_records_until_enabled() {
        let store = SamplingPointStore::disabled();
        store.record(SamplingPoint::new(10, true, vec![false]));
        assert!(store.is_empty());

        store.set_recording_enabled(true);
        store.record(SamplingPoint::new(20, false, vec![true]));
        assert_eq!(
            store.points_in_range(0, 30),
            vec![SamplingPoint::new(20, false, vec![true])]
        );
    }

    #[test]
    fn disabling_collection_preserves_existing_points() {
        let store = SamplingPointStore::default();
        store.record(SamplingPoint::new(10, true, vec![false]));

        store.set_recording_enabled(false);
        store.record(SamplingPoint::new(20, false, vec![true]));

        assert_eq!(
            store.points_in_range(0, 30),
            vec![SamplingPoint::new(10, true, vec![false])]
        );
    }

    #[test]
    fn provider_serves_ranges_without_enabling_recording() {
        let store = SamplingPointStore::disabled();
        store.set_provider(Arc::new(FixedProvider));
        store.set_recording_enabled(true);
        store.record(SamplingPoint::new(15, true, vec![true]));

        assert!(!store.is_recording_enabled());
        assert_eq!(
            store.points_in_range(10, 20),
            vec![
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(20, false, vec![true]),
            ]
        );
        assert!(
            store
                .points_in_range_with_minimum_spacing(10, 20, 11)
                .is_none()
        );
    }
}
