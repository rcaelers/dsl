//! Compact, indexed storage for decoded [`Word`](crate::events::Word) streams.
//!
//! Encoded blocks, indexes, queries, and cache publication are shared across
//! hosts; injected artifact repositories choose their physical backing.

mod backend;
mod cache;
mod codec;
mod config;
#[cfg(test)]
mod contract_tests;
mod errors;
mod format;
mod presence;
mod query;
mod state;
mod store;
mod vlq;

mod persistent;

pub(crate) use backend::{AnnotationStoreBackend, AnnotationStoreWriterBackend};
pub use cache::{
    DecodedBlockCacheStats, configure_decoded_block_cache, decoded_block_cache_stats,
    reset_decoded_block_cache_stats,
};
pub(crate) use codec::{EncodeWordBlockRequest, encode_owned_word_block};
pub use config::{BlockCodecConfig, LiveStoreConfig, PersistentStoreConfig};
pub use errors::{CodecError, CodecResult};
pub use persistent::{
    PersistentCacheClearTask, PersistentCacheEntrySnapshot, cleanup_cache, clear_cache,
    clear_cache_entry, inspect_cache_entry,
};
pub use query::{
    AnnotationQuery, AnnotationQueryError, AnnotationQueryResult, AnnotationStoreMetadata,
    ExactAnnotationWindow, WordPresenceBucket,
};
pub use state::{LiveStoreMetadata, StoreStatus};
pub use store::{
    CommittedAnnotationBlock, IndexedAnnotationStore, IndexedAnnotationWriter, StoreError,
    StoreResult,
};
