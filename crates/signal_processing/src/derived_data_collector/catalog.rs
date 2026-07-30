use std::sync::{Arc, RwLock};

use crate::payload::{
    CollectedLaneQuery, CollectedLaneSnapshotRequest, CollectedLaneStorageSnapshot,
    CollectedLaneTableMetadata, CollectedLaneTableSnapshot, OpaqueCollectedLaneSnapshot,
    PayloadDescriptor,
};

/// An adapter-owned retained query handle that generic consumers can discover
/// without knowing its concrete payload type.
#[derive(Clone)]
pub struct OpaqueCollectedLane {
    name: String,
    payload: PayloadDescriptor,
    query: Arc<dyn CollectedLaneQuery>,
}

impl OpaqueCollectedLane {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn payload(&self) -> &PayloadDescriptor {
        &self.payload
    }

    pub fn query<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        Arc::downcast::<T>(Arc::clone(&self.query).into_any()).ok()
    }

    /// Requests a bounded immutable snapshot for a presentation subscriber.
    pub fn snapshot(
        &self,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        self.query.snapshot(request)
    }

    /// Requests a nearby adapter-defined time boundary for generic cursor
    /// snapping without exposing the retained data representation.
    pub fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        self.query
            .nearest_time_boundary(timestamp_ns, max_distance_ns)
    }

    /// Returns the adapter-defined retained timeline extent without exposing
    /// the concrete lane storage.
    pub fn timeline_extent_end_ns(&self) -> Option<u64> {
        self.query.timeline_extent_end_ns()
    }

    pub fn is_live(&self) -> bool {
        self.query.is_live()
    }

    /// Returns table revision metadata supplied by the lane's adapter.
    pub fn table_metadata(&self) -> Option<CollectedLaneTableMetadata> {
        self.query.table_metadata()
    }

    /// Returns bounded table rows supplied by the lane's adapter.
    pub fn table_snapshot(&self, max_rows: usize) -> Option<CollectedLaneTableSnapshot> {
        self.query.table_snapshot(max_rows)
    }

    pub fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        self.query.storage_snapshot()
    }
}

impl std::fmt::Debug for OpaqueCollectedLane {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpaqueCollectedLane")
            .field("name", &self.name)
            .field("payload", &self.payload)
            .finish_non_exhaustive()
    }
}

/// Shared catalog of adapter-owned derived-lane queries. Producers and
/// subscribers hold independent clones, so subscribers may attach after
/// collection has begun or completed. A re-run swaps in a fresh catalog so
/// stale lanes vanish atomically.
#[derive(Debug, Clone, Default)]
pub struct DerivedLanes {
    opaque: Arc<RwLock<Vec<OpaqueCollectedLane>>>,
}

impl DerivedLanes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes an adapter-owned retained query. A later subscriber can
    /// attach after collection has completed and downcast only to the payload
    /// type it registered.
    pub fn publish_opaque_lane<T: CollectedLaneQuery + 'static>(
        &self,
        name: impl Into<String>,
        payload: PayloadDescriptor,
        query: Arc<T>,
    ) {
        let name = name.into();
        let lane = OpaqueCollectedLane {
            name: name.clone(),
            payload,
            query,
        };
        let mut lanes = self.opaque.write().unwrap();
        if let Some(index) = lanes.iter().position(|existing| existing.name == name) {
            lanes[index] = lane;
        } else {
            lanes.push(lane);
        }
    }

    pub fn opaque_lanes(&self) -> Vec<OpaqueCollectedLane> {
        self.opaque.read().unwrap().clone()
    }
}
