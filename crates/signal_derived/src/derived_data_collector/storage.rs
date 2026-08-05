use std::mem::size_of;

use crate::derived_index::MipmapRecord;
use crate::payload::{CollectedLaneStorageBacking, CollectedLaneStorageSnapshot};

pub(crate) fn in_memory_storage_snapshot<T>(
    retained_items: usize,
    index_items: usize,
) -> CollectedLaneStorageSnapshot {
    CollectedLaneStorageSnapshot {
        backing: CollectedLaneStorageBacking::Memory,
        retained_items: Some(retained_items as u64),
        resident_bytes: Some((retained_items * size_of::<T>()) as u64),
        stored_bytes: None,
        index_items: Some(index_items as u64),
        index_bytes: Some((index_items * size_of::<MipmapRecord>()) as u64),
        live: false,
    }
}
