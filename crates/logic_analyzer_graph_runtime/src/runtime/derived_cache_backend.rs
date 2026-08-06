use signal_derived::{DecodedBlockCacheHandle, IndexedAnnotationStore, PersistentStoreConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DerivedCacheLookup {
    Hit,
    Miss,
    Unreadable,
}

pub(crate) trait DerivedCacheBackend {
    fn lookup(&self, config: &PersistentStoreConfig) -> DerivedCacheLookup;
}

pub(crate) struct RepositoryDerivedCacheBackend {
    decoded_block_cache: DecodedBlockCacheHandle,
}

impl RepositoryDerivedCacheBackend {
    pub(crate) fn new(decoded_block_cache: DecodedBlockCacheHandle) -> Self {
        Self {
            decoded_block_cache,
        }
    }
}

impl DerivedCacheBackend for RepositoryDerivedCacheBackend {
    fn lookup(&self, config: &PersistentStoreConfig) -> DerivedCacheLookup {
        match IndexedAnnotationStore::open_persistent(config, self.decoded_block_cache.clone()) {
            Ok(Some(_)) => DerivedCacheLookup::Hit,
            Ok(None) => DerivedCacheLookup::Miss,
            Err(_) => DerivedCacheLookup::Unreadable,
        }
    }
}
