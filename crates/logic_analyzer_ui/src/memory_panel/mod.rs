//! Application diagnostics for retained and indexed signal data.

mod implementation;
mod model;
mod snapshot;

pub(crate) use implementation::MemoryPanel;
pub(crate) use model::{
    CaptureStorageBacking, CaptureStorageSnapshot, DerivedSignalStorageSnapshot,
    MemoryPanelSnapshot, MemoryServiceSnapshot,
};
