use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::contract::{
    ByteRange, ByteRegion, ImmutableByteRegion, SourceIdentity, SourceReadError,
};
use super::memory::OwnedByteSource;

/// Typed logical namespace for repository artifacts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactNamespace(String);

impl ArtifactNamespace {
    pub fn new(value: impl Into<String>) -> Result<Self, RepositoryError> {
        let value = value.into();
        if value.is_empty() {
            return Err(RepositoryError::InvalidKey(
                "artifact namespaces cannot be empty".into(),
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable repository address, independent of a filesystem path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactKey {
    namespace: ArtifactNamespace,
    identity: SourceIdentity,
}

impl ArtifactKey {
    pub fn new(namespace: ArtifactNamespace, identity: SourceIdentity) -> Self {
        Self {
            namespace,
            identity,
        }
    }

    pub fn namespace(&self) -> &ArtifactNamespace {
        &self.namespace
    }

    pub fn identity(&self) -> SourceIdentity {
        self.identity
    }
}

/// Information available without opening an artifact's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactMetadata {
    pub key: ArtifactKey,
    pub length: u64,
}

/// Storage properties used by cache policy and source preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepositoryCapabilities {
    pub durable: bool,
    pub atomic_publication: bool,
    pub immutable_regions: bool,
}

impl RepositoryCapabilities {
    pub const EPHEMERAL_MEMORY: Self = Self {
        durable: false,
        atomic_publication: true,
        immutable_regions: true,
    };
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RepositoryError {
    #[error("artifact repository is unavailable")]
    Unavailable,
    #[error("artifact repository quota is exhausted")]
    QuotaExceeded,
    #[error("artifact repository permission was denied")]
    PermissionDenied,
    #[error("artifact key is invalid: {0}")]
    InvalidKey(String),
    #[error("artifact range {offset}..+{length} overflows the address space")]
    RangeOverflow { offset: u64, length: u64 },
    #[error("artifact range {offset}..{end} exceeds artifact length {artifact_length}")]
    OutOfBounds {
        offset: u64,
        end: u64,
        artifact_length: u64,
    },
    #[error("artifact repository data is corrupt: {0}")]
    Corrupt(String),
    #[error("artifact repository I/O failed: {0}")]
    Io(String),
    #[error("artifact repository does not support {0}")]
    Unsupported(String),
}

impl From<SourceReadError> for RepositoryError {
    fn from(error: SourceReadError) -> Self {
        match error {
            SourceReadError::RangeOverflow { offset, length } => {
                Self::RangeOverflow { offset, length }
            }
            SourceReadError::OutOfBounds {
                offset,
                end,
                source_length,
            } => Self::OutOfBounds {
                offset,
                end,
                artifact_length: source_length,
            },
            SourceReadError::SourceChanged => Self::Corrupt("published artifact changed".into()),
            SourceReadError::Io(message) => Self::Io(message),
        }
    }
}

/// Immutable reader for one published artifact generation.
pub trait ReadArtifact: Send {
    fn key(&self) -> &ArtifactKey;

    fn len(&self) -> Result<u64, RepositoryError>;

    fn is_empty(&self) -> Result<bool, RepositoryError> {
        self.len().map(|length| length == 0)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, RepositoryError>;

    fn region(&self, range: ByteRange) -> Result<Option<ByteRegion>, RepositoryError>;
}

/// One unpublished artifact write. Dropping it never publishes incomplete data.
pub trait WriteArtifact: Send {
    fn key(&self) -> &ArtifactKey;

    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError>;

    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError>;

    fn flush(&mut self) -> Result<(), RepositoryError>;

    fn publish(self: Box<Self>) -> Result<(), RepositoryError>;
}

/// Repository of immutable published artifacts and private pending writes.
pub trait ArtifactRepository: Send + Sync {
    fn capabilities(&self) -> RepositoryCapabilities;

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError>;

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError>;

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError>;

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError>;
}

/// Portable process-lifetime repository with atomic in-memory publication.
#[derive(Clone, Default)]
pub struct MemoryArtifactRepository {
    artifacts: Arc<Mutex<BTreeMap<ArtifactKey, Arc<[u8]>>>>,
}

impl MemoryArtifactRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ArtifactRepository for MemoryArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities {
        RepositoryCapabilities::EPHEMERAL_MEMORY
    }

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError> {
        let bytes = self
            .artifacts
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?
            .get(key)
            .cloned();
        Ok(bytes.map(|bytes| {
            let backing: Arc<dyn ImmutableByteRegion> =
                Arc::new(OwnedByteSource::new(key.identity(), bytes));
            Box::new(MemoryReadArtifact {
                key: key.clone(),
                backing,
            }) as Box<dyn ReadArtifact>
        }))
    }

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError> {
        Ok(Box::new(MemoryWriteArtifact {
            repository: Arc::clone(&self.artifacts),
            key,
            bytes: Vec::new(),
        }))
    }

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError> {
        self.artifacts
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?
            .remove(key);
        Ok(())
    }

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError> {
        let artifacts = self
            .artifacts
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        Ok(artifacts
            .iter()
            .filter(|(key, _)| key.namespace() == namespace)
            .map(|(key, bytes)| ArtifactMetadata {
                key: key.clone(),
                length: bytes.len() as u64,
            })
            .collect())
    }
}

struct MemoryReadArtifact {
    key: ArtifactKey,
    backing: Arc<dyn ImmutableByteRegion>,
}

impl ReadArtifact for MemoryReadArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn len(&self) -> Result<u64, RepositoryError> {
        Ok(self.backing.len())
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, RepositoryError> {
        let length = self.backing.len();
        if offset > length {
            return Err(RepositoryError::OutOfBounds {
                offset,
                end: offset,
                artifact_length: length,
            });
        }
        let available = (length - offset) as usize;
        let count = available.min(destination.len());
        let range = ByteRange::new(offset, count as u64)?;
        destination[..count].copy_from_slice(self.backing.slice(range)?);
        Ok(count)
    }

    fn region(&self, range: ByteRange) -> Result<Option<ByteRegion>, RepositoryError> {
        Ok(Some(ByteRegion::new(Arc::clone(&self.backing), range)?))
    }
}

struct MemoryWriteArtifact {
    repository: Arc<Mutex<BTreeMap<ArtifactKey, Arc<[u8]>>>>,
    key: ArtifactKey,
    bytes: Vec<u8>,
}

impl WriteArtifact for MemoryWriteArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError> {
        let source_length =
            u64::try_from(source.len()).map_err(|_| RepositoryError::RangeOverflow {
                offset,
                length: u64::MAX,
            })?;
        let end = offset
            .checked_add(source_length)
            .ok_or(RepositoryError::RangeOverflow {
                offset,
                length: source_length,
            })?;
        let end = usize::try_from(end).map_err(|_| RepositoryError::RangeOverflow {
            offset,
            length: source_length,
        })?;
        if self.bytes.len() < end {
            self.bytes.resize(end, 0);
        }
        let start = usize::try_from(offset).map_err(|_| RepositoryError::RangeOverflow {
            offset,
            length: source_length,
        })?;
        self.bytes[start..end].copy_from_slice(source);
        Ok(())
    }

    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError> {
        let len = usize::try_from(len).map_err(|_| RepositoryError::RangeOverflow {
            offset: 0,
            length: len,
        })?;
        self.bytes.resize(len, 0);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn publish(self: Box<Self>) -> Result<(), RepositoryError> {
        self.repository
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?
            .insert(self.key.clone(), Arc::from(self.bytes));
        Ok(())
    }
}
