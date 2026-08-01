//! Compact, indexed storage for decoded [`Word`](crate::events::Word) streams.
//!
//! This module currently contains the versioned block format and its codec.
//! File lifecycle, live publication, and viewer queries are layered on top in
//! later implementation steps.

mod backend;
// The wasm store will consume the shared codec when it replaces its temporary
// in-memory word-vector implementation.
#[allow(
    dead_code,
    reason = "the wasm store has not adopted encoded blocks yet"
)]
mod codec;
mod config;
#[cfg(test)]
mod contract_tests;
mod errors;
// The wasm store will consume the shared format when it replaces its temporary
// in-memory word-vector implementation.
#[allow(
    dead_code,
    reason = "the wasm store has not adopted encoded blocks yet"
)]
mod format;
mod platform;
// The native store uses these detailed directory helpers today. Keeping them
// portable lets the wasm store use the same query algorithm.
#[allow(
    dead_code,
    reason = "the wasm store has not adopted encoded blocks yet"
)]
mod presence;
mod query;
mod state;
// The wasm store will consume the shared codec when it replaces its temporary
// in-memory word-vector implementation.
#[allow(
    dead_code,
    reason = "the wasm store has not adopted encoded blocks yet"
)]
mod vlq;

#[cfg(not(target_arch = "wasm32"))]
mod cache;
#[cfg(not(target_arch = "wasm32"))]
mod persistent;

pub(crate) use backend::{AnnotationStoreBackend, AnnotationStoreWriterBackend};
pub use config::{BlockCodecConfig, LiveStoreConfig, PersistentStoreConfig};
pub use errors::{CodecError, CodecResult};
#[cfg(not(target_arch = "wasm32"))]
pub use platform::{
    CommittedAnnotationBlock, DecodedBlockCacheStats, PersistentCacheEntrySnapshot, cleanup_cache,
    clear_cache, clear_cache_entry, configure_decoded_block_cache, decoded_block_cache_stats,
    inspect_cache_entry, reset_decoded_block_cache_stats,
};
pub use platform::{IndexedAnnotationStore, IndexedAnnotationWriter, StoreError, StoreResult};
pub use query::{
    AnnotationQuery, AnnotationQueryError, AnnotationQueryResult, AnnotationStoreMetadata,
    ExactAnnotationWindow, WordPresenceBucket,
};
pub use state::{LiveStoreMetadata, StoreStatus};
