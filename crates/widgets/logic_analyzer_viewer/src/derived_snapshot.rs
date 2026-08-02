use std::collections::HashMap;
use std::time::Duration;

use web_time::Instant;

use signal_processing::{
    CollectedLaneSnapshotRequest, OpaqueCollectedLane, OpaqueCollectedLaneSnapshot,
};

const LIVE_REFRESH_INTERVAL: Duration = Duration::from_millis(50);
const MAX_REQUESTS_PER_LANE: usize = 2;

#[derive(Default)]
pub(crate) struct DerivedSnapshotCache {
    entries: HashMap<String, Vec<DerivedSnapshotCacheEntry>>,
}

struct DerivedSnapshotCacheEntry {
    lane: OpaqueCollectedLane,
    request: CollectedLaneSnapshotRequest,
    generation: u64,
    sampled_at: Instant,
    snapshot: Option<OpaqueCollectedLaneSnapshot>,
}

impl DerivedSnapshotCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn snapshot(
        &mut self,
        lane: &OpaqueCollectedLane,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        self.snapshot_at(lane, request, Instant::now())
    }

    fn snapshot_at(
        &mut self,
        lane: &OpaqueCollectedLane,
        request: CollectedLaneSnapshotRequest,
        now: Instant,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        let Some(generation) = lane.snapshot_generation() else {
            self.entries.remove(lane.name());
            return lane.snapshot(request);
        };
        let entries = self.entries.entry(lane.name().to_owned()).or_default();
        if entries
            .first()
            .is_some_and(|cached| !cached.lane.is_same_query(lane))
        {
            entries.clear();
        }
        if let Some(cached) = entries.iter().find(|cached| cached.request == request)
            && (cached.generation == generation
                || (lane.is_live()
                    && now.duration_since(cached.sampled_at) < LIVE_REFRESH_INTERVAL))
        {
            return cached.snapshot.clone();
        }

        let snapshot = lane.snapshot(request);
        if snapshot.is_none()
            && let Some(cached) = entries.iter().find(|cached| cached.request == request)
        {
            // A live adapter may decline a query rather than wait for its
            // writer. Keep the last immutable snapshot so a busy collector
            // can never stall or blank the UI thread.
            return cached.snapshot.clone();
        }
        if let Some(cached) = entries.iter_mut().find(|cached| cached.request == request) {
            *cached = DerivedSnapshotCacheEntry {
                lane: lane.clone(),
                request,
                generation,
                sampled_at: now,
                snapshot: snapshot.clone(),
            };
        } else {
            if entries.len() >= MAX_REQUESTS_PER_LANE {
                let oldest = entries
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, cached)| cached.sampled_at)
                    .map(|(index, _)| index)
                    .unwrap_or(0);
                entries.remove(oldest);
            }
            entries.push(DerivedSnapshotCacheEntry {
                lane: lane.clone(),
                request,
                generation,
                sampled_at: now,
                snapshot: snapshot.clone(),
            });
        }
        snapshot
    }
}

#[cfg(test)]
mod derived_snapshot_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use signal_processing::{
        CollectedLaneQuery, DerivedLanes, OpaqueCollectedLaneSnapshot, PayloadRegistry,
    };

    use super::*;

    const PAYLOAD_ID: &str = "org.logicconduit.test.snapshot-cache/v1";

    #[derive(Clone)]
    struct TestPayload;

    struct TestQuery {
        generation: Option<Arc<AtomicU64>>,
        live: AtomicBool,
        snapshots: AtomicU64,
    }

    struct BusyQuery {
        generation: AtomicU64,
        available: AtomicBool,
    }

    impl CollectedLaneQuery for BusyQuery {
        fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
            self
        }

        fn snapshot_generation(&self) -> Option<u64> {
            Some(self.generation.load(Ordering::Acquire))
        }

        fn snapshot(
            &self,
            _request: CollectedLaneSnapshotRequest,
        ) -> Option<OpaqueCollectedLaneSnapshot> {
            self.available
                .load(Ordering::Acquire)
                .then(|| OpaqueCollectedLaneSnapshot::new(Arc::new(42_u64)))
        }

        fn is_live(&self) -> bool {
            true
        }
    }

    impl CollectedLaneQuery for TestQuery {
        fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
            self
        }

        fn snapshot_generation(&self) -> Option<u64> {
            self.generation
                .as_ref()
                .map(|generation| generation.load(Ordering::Acquire))
        }

        fn snapshot(
            &self,
            _request: CollectedLaneSnapshotRequest,
        ) -> Option<OpaqueCollectedLaneSnapshot> {
            let count = self.snapshots.fetch_add(1, Ordering::AcqRel) + 1;
            Some(OpaqueCollectedLaneSnapshot::new(Arc::new(count)))
        }

        fn is_live(&self) -> bool {
            self.live.load(Ordering::Acquire)
        }
    }

    fn published_lane<Q>(query: Arc<Q>) -> OpaqueCollectedLane
    where
        Q: CollectedLaneQuery + 'static,
    {
        let mut payloads = PayloadRegistry::new();
        payloads.register::<TestPayload>(PAYLOAD_ID).unwrap();
        let lanes = DerivedLanes::new();
        lanes.publish_opaque_lane(
            "lane",
            payloads.descriptor::<TestPayload>().unwrap().clone(),
            query,
        );
        lanes.opaque_lanes().remove(0)
    }

    fn request(max_items: usize) -> CollectedLaneSnapshotRequest {
        CollectedLaneSnapshotRequest {
            start_time_ns: 10,
            end_time_ns: 20,
            max_items,
        }
    }

    #[test]
    fn stable_windows_reuse_snapshots_and_live_revisions_are_rate_limited() {
        let generation = Arc::new(AtomicU64::new(0));
        let query = Arc::new(TestQuery {
            generation: Some(Arc::clone(&generation)),
            live: AtomicBool::new(true),
            snapshots: AtomicU64::new(0),
        });
        let lane = published_lane(Arc::clone(&query));
        let mut cache = DerivedSnapshotCache::new();
        let started = Instant::now();

        cache.snapshot_at(&lane, request(100), started);
        cache.snapshot_at(&lane, request(100), started + Duration::from_millis(10));
        generation.store(1, Ordering::Release);
        cache.snapshot_at(&lane, request(100), started + Duration::from_millis(20));
        assert_eq!(query.snapshots.load(Ordering::Acquire), 1);

        cache.snapshot_at(&lane, request(100), started + LIVE_REFRESH_INTERVAL);
        cache.snapshot_at(&lane, request(200), started + LIVE_REFRESH_INTERVAL);
        assert_eq!(query.snapshots.load(Ordering::Acquire), 3);

        query.live.store(false, Ordering::Release);
        generation.store(2, Ordering::Release);
        cache.snapshot_at(
            &lane,
            request(200),
            started + LIVE_REFRESH_INTERVAL + Duration::from_millis(1),
        );
        assert_eq!(query.snapshots.load(Ordering::Acquire), 4);
        cache.snapshot_at(&lane, request(300), started + Duration::from_secs(1));
        assert_eq!(cache.entries["lane"].len(), MAX_REQUESTS_PER_LANE);
    }

    #[test]
    fn unversioned_and_replaced_queries_never_reuse_stale_snapshots() {
        let unversioned = Arc::new(TestQuery {
            generation: None,
            live: AtomicBool::new(false),
            snapshots: AtomicU64::new(0),
        });
        let lane = published_lane(Arc::clone(&unversioned));
        let mut cache = DerivedSnapshotCache::new();
        let started = Instant::now();
        cache.snapshot_at(&lane, request(100), started);
        cache.snapshot_at(&lane, request(100), started);
        assert_eq!(unversioned.snapshots.load(Ordering::Acquire), 2);

        let first = Arc::new(TestQuery {
            generation: Some(Arc::new(AtomicU64::new(7))),
            live: AtomicBool::new(false),
            snapshots: AtomicU64::new(0),
        });
        let replacement = Arc::new(TestQuery {
            generation: Some(Arc::new(AtomicU64::new(7))),
            live: AtomicBool::new(false),
            snapshots: AtomicU64::new(0),
        });
        cache.snapshot_at(&published_lane(first), request(100), started);
        cache.snapshot_at(
            &published_lane(Arc::clone(&replacement)),
            request(100),
            started,
        );
        assert_eq!(replacement.snapshots.load(Ordering::Acquire), 1);
    }

    #[test]
    fn busy_live_query_keeps_the_last_complete_snapshot() {
        let query = Arc::new(BusyQuery {
            generation: AtomicU64::new(0),
            available: AtomicBool::new(true),
        });
        let lane = published_lane(Arc::clone(&query));
        let mut cache = DerivedSnapshotCache::new();
        let started = Instant::now();
        let first = cache
            .snapshot_at(&lane, request(100), started)
            .and_then(|snapshot| snapshot.value::<u64>())
            .expect("initial complete snapshot");
        assert_eq!(*first, 42);

        query.available.store(false, Ordering::Release);
        query.generation.store(1, Ordering::Release);
        let retained = cache
            .snapshot_at(&lane, request(100), started + LIVE_REFRESH_INTERVAL)
            .and_then(|snapshot| snapshot.value::<u64>())
            .expect("cached snapshot while the writer is busy");
        assert_eq!(*retained, 42);
    }
}
