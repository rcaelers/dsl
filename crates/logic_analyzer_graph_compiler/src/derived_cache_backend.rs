use std::path::Path;

use signal_processing::PersistentStoreConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DerivedCacheLookup {
    Hit,
    Miss,
    Unreadable,
}

pub(crate) trait DerivedCacheBackend {
    fn cleanup(
        &self,
        directory: &Path,
        max_total_bytes: u64,
        pinned_keys: &[[u8; 32]],
    ) -> Result<(), String>;

    fn lookup(&self, config: &PersistentStoreConfig) -> DerivedCacheLookup;
}
