//! Native file-backed and mmap-backed derived-word storage.

pub use super::super::persistent::{
    PersistentCacheEntrySnapshot, cleanup_cache, clear_cache, clear_cache_entry,
    inspect_cache_entry,
};
#[path = "../../store.rs"]
mod store;

pub(crate) use store::default_working_directory;
pub use store::{
    CommittedAnnotationBlock, IndexedAnnotationStore, IndexedAnnotationWriter, StoreError,
    StoreResult,
};
