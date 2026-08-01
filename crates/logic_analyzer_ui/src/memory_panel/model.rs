use signal_processing::CollectedLaneStorageSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureStorageBacking {
    InMemory,
    BuildingIndex,
    Indexed,
    GrowingIndex,
    MetadataOnly,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CaptureStorageSnapshot {
    pub(crate) name: String,
    pub(crate) status: String,
    pub(crate) backing: CaptureStorageBacking,
    pub(crate) channels: usize,
    pub(crate) total_samples: Option<u64>,
    pub(crate) data_bytes: Option<u64>,
    pub(crate) index_identity: Option<String>,
    pub(crate) index_progress: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DerivedSignalStorageSnapshot {
    pub(crate) name: String,
    pub(crate) payload_id: String,
    pub(crate) storage: CollectedLaneStorageSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemoryServiceSnapshot {
    pub(crate) name: String,
    pub(crate) state: String,
    pub(crate) detail: String,
    pub(crate) used_bytes: Option<u64>,
    pub(crate) budget_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PersistentCacheSnapshotState {
    Ready,
    Missing,
    Unreadable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PersistentCacheSnapshot {
    pub(crate) cache_key: [u8; 32],
    pub(crate) owners: Vec<String>,
    pub(crate) repository: String,
    pub(crate) state: PersistentCacheSnapshotState,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) data_bytes: Option<u64>,
    pub(crate) index_bytes: Option<u64>,
    pub(crate) items: Option<u64>,
    pub(crate) index_items: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CacheMemorySnapshot {
    pub(crate) services: Vec<MemoryServiceSnapshot>,
    pub(crate) persistent_caches: Vec<PersistentCacheSnapshot>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MemoryPanelSnapshot {
    pub(crate) services: Vec<MemoryServiceSnapshot>,
    pub(crate) capture: Option<CaptureStorageSnapshot>,
    pub(crate) derived_lanes: Vec<DerivedSignalStorageSnapshot>,
    pub(crate) persistent_caches: Vec<PersistentCacheSnapshot>,
}
