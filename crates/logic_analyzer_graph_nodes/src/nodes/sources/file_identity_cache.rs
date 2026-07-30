use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

struct CachedIdentity {
    stamp: FileStamp,
    identity: [u8; 32],
}

/// Retains expensive format-derived identities while the underlying file is unchanged.
///
/// Graph discovery lowers an unchanged document for several independent UI services. File
/// sources should not reopen and parse a multi-gigabyte capture for every one of those reads.
#[derive(Default)]
pub(crate) struct FileIdentityCache {
    entries: Mutex<HashMap<PathBuf, CachedIdentity>>,
}

impl FileIdentityCache {
    pub(crate) fn resolve(
        &self,
        path: &Path,
        load: impl FnOnce(&Path) -> Result<[u8; 32], String>,
    ) -> Result<[u8; 32], String> {
        let stamp = FileStamp::read(path);
        if let Some(stamp) = &stamp
            && let Some(cached) = self.entries.lock().unwrap().get(path)
            && cached.stamp == *stamp
        {
            return Ok(cached.identity);
        }

        let identity = load(path)?;
        if let Some(stamp) = stamp {
            self.entries
                .lock()
                .unwrap()
                .insert(path.to_owned(), CachedIdentity { stamp, identity });
        }
        Ok(identity)
    }
}

#[cfg(test)]
mod file_identity_cache_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn identity_is_reused_until_the_file_changes() {
        let path = std::env::temp_dir().join(format!(
            "logic-conduit-file-identity-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"capture").unwrap();
        let cache = FileIdentityCache::default();
        let loads = AtomicUsize::new(0);
        let load = |_: &Path| {
            loads.fetch_add(1, Ordering::Relaxed);
            Ok([0x42; 32])
        };

        assert_eq!(cache.resolve(&path, load).unwrap(), [0x42; 32]);
        assert_eq!(cache.resolve(&path, load).unwrap(), [0x42; 32]);
        assert_eq!(loads.load(Ordering::Relaxed), 1);

        std::fs::write(&path, b"changed capture").unwrap();
        assert_eq!(cache.resolve(&path, load).unwrap(), [0x42; 32]);
        assert_eq!(loads.load(Ordering::Relaxed), 2);

        std::fs::remove_file(path).unwrap();
    }
}
