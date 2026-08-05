use signal_derived::{IndexedAnnotationStore, PersistentStoreConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DerivedCacheLookup {
    Hit,
    Miss,
    Unreadable,
}

pub(crate) trait DerivedCacheBackend {
    fn lookup(&self, config: &PersistentStoreConfig) -> DerivedCacheLookup;
}

pub(crate) struct RepositoryDerivedCacheBackend;

impl DerivedCacheBackend for RepositoryDerivedCacheBackend {
    fn lookup(&self, config: &PersistentStoreConfig) -> DerivedCacheLookup {
        match IndexedAnnotationStore::open_persistent(config) {
            Ok(Some(_)) => DerivedCacheLookup::Hit,
            Ok(None) => DerivedCacheLookup::Miss,
            Err(_) => DerivedCacheLookup::Unreadable,
        }
    }
}
