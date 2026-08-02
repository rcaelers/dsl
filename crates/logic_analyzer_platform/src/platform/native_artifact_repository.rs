use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::MmapOptions;

use signal_processing::{
    ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactRepository, ByteRange, ByteRegion,
    ImmutableByteRegion, ReadArtifact, RepositoryCapabilities, RepositoryError, SourceIdentity,
    WriteArtifact,
};

/// Durable native artifact repository selected at the platform boundary.
pub(crate) struct NativeArtifactRepository {
    root: PathBuf,
    next_temporary_id: AtomicU64,
}

impl NativeArtifactRepository {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            next_temporary_id: AtomicU64::new(1),
        }
    }

    fn namespace_directory(&self, namespace: &ArtifactNamespace) -> PathBuf {
        self.root.join(hex_encode(namespace.as_str().as_bytes()))
    }

    fn artifact_path(&self, key: &ArtifactKey) -> PathBuf {
        self.namespace_directory(key.namespace())
            .join(hex_encode(key.identity().as_bytes()))
    }

    fn temporary_path(&self, key: &ArtifactKey) -> PathBuf {
        let id = self.next_temporary_id.fetch_add(1, Ordering::Relaxed);
        self.temporary_path_with_id(key, id)
    }

    fn temporary_path_with_id(&self, key: &ArtifactKey, id: u64) -> PathBuf {
        let artifact_name = hex_encode(key.identity().as_bytes());
        self.namespace_directory(key.namespace())
            .join(format!(".{artifact_name}.{id}.pending"))
    }

    fn create_temporary_artifact(
        &self,
        key: &ArtifactKey,
    ) -> Result<(PathBuf, File), RepositoryError> {
        loop {
            let temporary_path = self.temporary_path(key);
            match OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&temporary_path)
            {
                Ok(file) => return Ok((temporary_path, file)),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(repository_io(error)),
            }
        }
    }
}

impl ArtifactRepository for NativeArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities {
        RepositoryCapabilities {
            durable: true,
            atomic_publication: true,
            immutable_regions: true,
        }
    }

    fn namespaces(&self) -> Result<Vec<ArtifactNamespace>, RepositoryError> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(repository_io(error)),
        };
        let mut namespaces = Vec::new();
        for entry in entries {
            let entry = entry.map_err(repository_io)?;
            if !entry.file_type().map_err(repository_io)?.is_dir() {
                continue;
            }
            let Some(encoded) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(bytes) = hex_decode(&encoded) else {
                continue;
            };
            let Ok(name) = String::from_utf8(bytes) else {
                continue;
            };
            if let Ok(namespace) = ArtifactNamespace::new(name) {
                namespaces.push(namespace);
            }
        }
        namespaces.sort();
        Ok(namespaces)
    }

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError> {
        let path = self.artifact_path(key);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(repository_io(error)),
        };
        let length = file.metadata().map_err(repository_io)?.len();
        let backing = if length == 0 {
            NativeArtifactRegion::Empty
        } else {
            // SAFETY: published artifacts are immutable and the mapping owns
            // its file-backed pages for the lifetime of this read generation.
            let map = unsafe { MmapOptions::new().map(&file) }.map_err(repository_io)?;
            NativeArtifactRegion::Mapped(map)
        };
        Ok(Some(Box::new(NativeReadArtifact {
            key: key.clone(),
            backing: Arc::new(backing),
            length,
        })))
    }

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError> {
        let directory = self.namespace_directory(key.namespace());
        std::fs::create_dir_all(&directory).map_err(repository_io)?;
        let (temporary_path, file) = self.create_temporary_artifact(&key)?;
        Ok(Box::new(NativeWriteArtifact {
            final_path: self.artifact_path(&key),
            key,
            file: Some(file),
            temporary_path,
            published: false,
            flushed: false,
        }))
    }

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError> {
        match std::fs::remove_file(self.artifact_path(key)) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(repository_io(error)),
        }
        let _ = std::fs::remove_dir(self.namespace_directory(key.namespace()));
        Ok(())
    }

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError> {
        let entries = match std::fs::read_dir(self.namespace_directory(namespace)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(repository_io(error)),
        };
        let mut artifacts = Vec::new();
        for entry in entries {
            let entry = entry.map_err(repository_io)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(identity) = parse_identity(&name) else {
                continue;
            };
            let metadata = entry.metadata().map_err(repository_io)?;
            if metadata.is_file() {
                artifacts.push(ArtifactMetadata {
                    key: ArtifactKey::new(namespace.clone(), identity),
                    length: metadata.len(),
                });
            }
        }
        artifacts.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(artifacts)
    }
}

enum NativeArtifactRegion {
    Mapped(memmap2::Mmap),
    Empty,
}

impl ImmutableByteRegion for NativeArtifactRegion {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Mapped(map) => map,
            Self::Empty => &[],
        }
    }
}

struct NativeReadArtifact {
    key: ArtifactKey,
    backing: Arc<NativeArtifactRegion>,
    length: u64,
}

impl ReadArtifact for NativeReadArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn len(&self) -> Result<u64, RepositoryError> {
        Ok(self.length)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, RepositoryError> {
        if offset > self.length {
            return Err(RepositoryError::OutOfBounds {
                offset,
                end: offset,
                artifact_length: self.length,
            });
        }
        let start = usize::try_from(offset).map_err(|_| RepositoryError::OutOfBounds {
            offset,
            end: offset,
            artifact_length: self.length,
        })?;
        let source = &self.backing.bytes()[start..];
        let count = source.len().min(destination.len());
        destination[..count].copy_from_slice(&source[..count]);
        Ok(count)
    }

    fn region(&self, range: ByteRange) -> Result<Option<ByteRegion>, RepositoryError> {
        if range.end() > self.length {
            return Err(RepositoryError::OutOfBounds {
                offset: range.offset,
                end: range.end(),
                artifact_length: self.length,
            });
        }
        let backing: Arc<dyn ImmutableByteRegion> = self.backing.clone();
        ByteRegion::new(backing, range)
            .map(Some)
            .map_err(RepositoryError::from)
    }
}

struct NativeWriteArtifact {
    key: ArtifactKey,
    file: Option<File>,
    temporary_path: PathBuf,
    final_path: PathBuf,
    published: bool,
    flushed: bool,
}

impl NativeWriteArtifact {
    fn file_mut(&mut self) -> Result<&mut File, RepositoryError> {
        self.file
            .as_mut()
            .ok_or_else(|| RepositoryError::Io("artifact write was already published".into()))
    }
}

impl WriteArtifact for NativeWriteArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError> {
        self.flushed = false;
        self.file_mut()?
            .seek(SeekFrom::Start(offset))
            .map_err(repository_io)?;
        self.file_mut()?.write_all(source).map_err(repository_io)
    }

    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError> {
        self.flushed = false;
        self.file_mut()?.set_len(len).map_err(repository_io)
    }

    fn flush(&mut self) -> Result<(), RepositoryError> {
        self.file_mut()?.sync_all().map_err(repository_io)?;
        self.flushed = true;
        Ok(())
    }

    fn publish(mut self: Box<Self>) -> Result<(), RepositoryError> {
        let mut file = self
            .file
            .take()
            .ok_or_else(|| RepositoryError::Io("artifact write was already published".into()))?;
        if !self.flushed {
            // Publication still closes the complete file before the atomic
            // rename, but rebuildable caches can deliberately omit the
            // durability barrier exposed by `flush`.
            file.flush().map_err(repository_io)?;
        }
        drop(file);
        std::fs::rename(&self.temporary_path, &self.final_path).map_err(repository_io)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for NativeWriteArtifact {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.temporary_path);
            if let Some(directory) = self.temporary_path.parent() {
                let _ = std::fs::remove_dir(directory);
            }
        }
    }
}

fn repository_io(error: std::io::Error) -> RepositoryError {
    RepositoryError::Io(error.to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    let (pairs, []) = value.as_bytes().as_chunks::<2>() else {
        return None;
    };
    pairs
        .iter()
        .map(|pair| Some((hex_value(pair[0])? << 4) | hex_value(pair[1])?))
        .collect()
}

fn parse_identity(value: &str) -> Option<SourceIdentity> {
    if value.len() != 64 {
        return None;
    }
    let mut bytes = [0_u8; 32];
    let (pairs, []) = value.as_bytes().as_chunks::<2>() else {
        return None;
    };
    for (index, pair) in pairs.iter().enumerate() {
        bytes[index] = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
    }
    Some(SourceIdentity::from_bytes(bytes))
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod native_artifact_repository_tests {
    use logic_analyzer_test_support::{
        capture_store_conformance, derived_store_conformance, repository_conformance,
    };
    use signal_processing::MemoryArtifactRepository;

    use super::*;

    #[test]
    fn repository_satisfies_the_shared_artifact_and_source_contracts() {
        let directory = tempfile::tempdir().unwrap();
        let native: Arc<dyn ArtifactRepository> = Arc::new(NativeArtifactRepository::new(
            directory.path().join("artifacts"),
        ));
        repository_conformance(
            native,
            RepositoryCapabilities {
                durable: true,
                atomic_publication: true,
                immutable_regions: true,
            },
        );

        let memory: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        repository_conformance(memory, RepositoryCapabilities::EPHEMERAL_MEMORY);
    }

    #[test]
    fn repository_skips_pending_names_left_by_an_interrupted_process() {
        let directory = tempfile::tempdir().unwrap();
        let repository = NativeArtifactRepository::new(directory.path().join("artifacts"));
        let namespace = ArtifactNamespace::new("restart collision").unwrap();
        let key = ArtifactKey::new(namespace.clone(), SourceIdentity::from_bytes([0x31; 32]));
        let namespace_directory = repository.namespace_directory(&namespace);
        std::fs::create_dir_all(&namespace_directory).unwrap();
        let abandoned = repository.temporary_path_with_id(&key, 1);
        File::create(&abandoned).unwrap();

        let mut writer = repository.begin_write(key.clone()).unwrap();
        writer.write_at(0, b"recovered").unwrap();
        writer.publish().unwrap();

        let mut published = repository.open(&key).unwrap().unwrap();
        let mut bytes = [0_u8; 9];
        published.read_at(0, &mut bytes).unwrap();
        assert_eq!(&bytes, b"recovered");
        assert!(abandoned.exists());
    }

    #[test]
    fn native_and_memory_repositories_run_the_same_derived_store() {
        let directory = tempfile::tempdir().unwrap();
        let native: Arc<dyn ArtifactRepository> = Arc::new(NativeArtifactRepository::new(
            directory.path().join("artifacts"),
        ));
        let memory: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());

        assert_eq!(
            derived_store_conformance(native),
            derived_store_conformance(memory)
        );
    }

    #[test]
    fn native_and_memory_repositories_run_the_same_capture_store() {
        let directory = tempfile::tempdir().unwrap();
        let native: Arc<dyn ArtifactRepository> = Arc::new(NativeArtifactRepository::new(
            directory.path().join("artifacts"),
        ));
        let memory: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());

        assert_eq!(
            capture_store_conformance(native),
            capture_store_conformance(memory)
        );
    }
}
