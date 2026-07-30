//! Application diagnostics for retained and indexed signal data.

mod implementation;
mod model;

pub(crate) use implementation::MemoryPanel;
pub(crate) use model::{
    CaptureStorageBacking, CaptureStorageSnapshot, DerivedSignalStorageSnapshot,
    MemoryPanelSnapshot, MemoryServiceSnapshot, PersistentCacheSnapshot,
    PersistentCacheSnapshotState, PlatformMemorySnapshot,
};
