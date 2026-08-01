use std::sync::Arc;

use signal_processing::{ArtifactRepository, IndexedAnnotationStore, PersistentStoreConfig};

use super::derived_cache_backend::{DerivedCacheBackend, DerivedCacheLookup};

pub(crate) struct NativeDerivedCacheBackend;

impl DerivedCacheBackend for NativeDerivedCacheBackend {
    fn cleanup(
        &self,
        repository: &Arc<dyn ArtifactRepository>,
        max_total_bytes: u64,
        pinned_keys: &[[u8; 32]],
    ) -> Result<(), String> {
        signal_processing::cleanup_cache(repository, max_total_bytes, pinned_keys)
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
