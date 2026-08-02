use std::sync::Arc;

use signal_processing::{ArtifactRepository, IndexedAnnotationStore, PersistentStoreConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DerivedCacheLookup {
    Hit,
    Miss,
    Unreadable,
}

pub(crate) trait DerivedCacheBackend {
    fn cleanup(
        &self,
        repository: &Arc<dyn ArtifactRepository>,
        max_total_bytes: u64,
        pinned_keys: &[[u8; 32]],
    ) -> Result<(), String>;

    fn lookup(&self, config: &PersistentStoreConfig) -> DerivedCacheLookup;
}

pub(crate) struct RepositoryDerivedCacheBackend;

impl DerivedCacheBackend for RepositoryDerivedCacheBackend {
    fn cleanup(
        &self,
        repository: &Arc<dyn ArtifactRepository>,
        max_total_bytes: u64,
        pinned_keys: &[[u8; 32]],
    ) -> Result<(), String> {
        signal_processing::derived_word_store::cleanup_cache(
            repository,
            max_total_bytes,
            pinned_keys,
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    fn lookup(&self, config: &PersistentStoreConfig) -> DerivedCacheLookup {
        match IndexedAnnotationStore::open_persistent(config) {
            Ok(Some(_)) => DerivedCacheLookup::Hit,
            Ok(None) => DerivedCacheLookup::Miss,
            Err(_) => DerivedCacheLookup::Unreadable,
        }
    }
}
