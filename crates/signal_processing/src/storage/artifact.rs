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
    /// Creates a non-empty logical artifact namespace.
    ///
    /// # Parameters
    /// - `value`: Stable logical namespace, independent of host paths.
    pub fn new(value: impl Into<String>) -> Result<Self, RepositoryError> {
        let value = value.into();
        if value.is_empty() {
            return Err(RepositoryError::InvalidKey(
                "artifact namespaces cannot be empty".into(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the namespace's stable string value.
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
    /// Creates an artifact address from namespace and content identity.
    pub fn new(namespace: ArtifactNamespace, identity: SourceIdentity) -> Self {
        Self {
            namespace,
            identity,
        }
    }

    /// Returns the logical namespace portion of this address.
    pub fn namespace(&self) -> &ArtifactNamespace {
        &self.namespace
    }

    /// Returns the content-oriented identity portion of this address.
    pub fn identity(&self) -> SourceIdentity {
        self.identity
    }
}

/// Information available without opening an artifact's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactMetadata {
    /// Stable repository address of the artifact.
    pub key: ArtifactKey,
    /// Published artifact length in bytes.
    pub length: u64,
}

/// Storage properties used by cache policy and source preparation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepositoryCapabilities {
    /// Whether artifacts survive process or host restarts.
    pub durable: bool,
    /// Whether readers observe a complete generation or no publication.
    pub atomic_publication: bool,
    /// Whether opened byte regions remain stable through later publication.
    pub immutable_regions: bool,
}

impl RepositoryCapabilities {
    /// Capabilities of the in-memory repository implementation.
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
    /// A requested range cannot be represented without overflowing `u64`.
    RangeOverflow {
        /// Starting byte offset.
        offset: u64,
        /// Requested byte count.
        length: u64,
    },
    #[error("artifact range {offset}..{end} exceeds artifact length {artifact_length}")]
    /// A requested range extends past the published artifact length.
    OutOfBounds {
        /// Starting byte offset.
        offset: u64,
        /// First byte after the requested range.
        end: u64,
        /// Published artifact length.
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
    /// Returns the stable address of the opened artifact generation.
    fn key(&self) -> &ArtifactKey;

    /// Returns the published byte length.
    fn len(&self) -> Result<u64, RepositoryError>;

    /// Returns whether the artifact contains no bytes.
    fn is_empty(&self) -> Result<bool, RepositoryError> {
        self.len().map(|length| length == 0)
    }

    /// Reads bytes beginning at an absolute artifact offset.
    ///
    /// # Parameters
    /// - `offset`: Absolute byte offset in the artifact.
    /// - `destination`: Buffer receiving the read bytes.
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, RepositoryError>;

    /// Opens an immutable region for a valid artifact byte range.
    fn region(&self, range: ByteRange) -> Result<Option<ByteRegion>, RepositoryError>;
}

/// One unpublished artifact write. Dropping it never publishes incomplete data.
pub trait WriteArtifact: Send {
    /// Returns the stable address reserved for this pending write.
    fn key(&self) -> &ArtifactKey;

    /// Writes bytes at an absolute artifact offset.
    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError>;

    /// Sets the pending artifact's byte length.
    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError>;

    /// Flushes pending data without making it visible to readers.
    fn flush(&mut self) -> Result<(), RepositoryError>;

    /// Atomically publishes this completed artifact generation.
    fn publish(self: Box<Self>) -> Result<(), RepositoryError>;
}

/// Repository of immutable published artifacts and private pending writes.
pub trait ArtifactRepository: Send + Sync {
    /// Returns storage and region-access capabilities.
    fn capabilities(&self) -> RepositoryCapabilities;

    /// Lists namespaces containing published artifacts.
    fn namespaces(&self) -> Result<Vec<ArtifactNamespace>, RepositoryError>;

    /// Opens one published artifact generation by key.
    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError>;

    /// Begins an unpublished write at a stable artifact address.
    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError>;

    /// Removes the published artifact addressed by `key`.
    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError>;

    /// Lists metadata for artifacts published in one namespace.
    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError>;
}

const DEFAULT_MEMORY_CHUNK_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct MemoryRepositoryState {
    artifacts: BTreeMap<ArtifactKey, MemoryArtifact>,
    used_bytes: u64,
}

#[derive(Clone)]
struct MemoryArtifact {
    chunks: Arc<[Arc<[u8]>]>,
    len: u64,
}

/// Portable process-lifetime repository with bounded, chunked memory and
/// atomic publication.
#[derive(Clone)]
pub struct MemoryArtifactRepository {
    state: Arc<Mutex<MemoryRepositoryState>>,
    max_bytes: u64,
    chunk_bytes: usize,
}

impl MemoryArtifactRepository {
    /// Creates an empty in-memory immutable-artifact repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns this value configured with budget.
    ///
    /// # Parameters
    /// - `max_bytes`: Input consumed by this operation.
    pub fn with_budget(max_bytes: u64) -> Self {
        Self {
            max_bytes,
            ..Self::default()
        }
    }

    /// Returns this value configured with budget and chunk size.
    pub fn with_budget_and_chunk_size(
        max_bytes: u64,
        chunk_bytes: usize,
    ) -> Result<Self, RepositoryError> {
        if chunk_bytes == 0 {
            return Err(RepositoryError::Unsupported(
                "a zero-sized memory artifact chunk".into(),
            ));
        }
        Ok(Self {
            state: Arc::new(Mutex::new(MemoryRepositoryState::default())),
            max_bytes,
            chunk_bytes,
        })
    }

    /// Returns the total bytes occupied by published in-memory artifacts.
    pub fn used_bytes(&self) -> Result<u64, RepositoryError> {
        self.state
            .lock()
            .map(|state| state.used_bytes)
            .map_err(|_| RepositoryError::Unavailable)
    }
}

impl Default for MemoryArtifactRepository {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(MemoryRepositoryState::default())),
            max_bytes: u64::MAX,
            chunk_bytes: DEFAULT_MEMORY_CHUNK_BYTES,
        }
    }
}

impl ArtifactRepository for MemoryArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities {
        RepositoryCapabilities::EPHEMERAL_MEMORY
    }

    fn namespaces(&self) -> Result<Vec<ArtifactNamespace>, RepositoryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        Ok(state
            .artifacts
            .keys()
            .map(|key| key.namespace().clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError> {
        let artifact = self
            .state
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?
            .artifacts
            .get(key)
            .cloned();
        Ok(artifact.map(|artifact| {
            Box::new(MemoryReadArtifact {
                key: key.clone(),
                artifact,
                chunk_bytes: self.chunk_bytes,
            }) as Box<dyn ReadArtifact>
        }))
    }

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError> {
        Ok(Box::new(MemoryWriteArtifact {
            repository: Arc::clone(&self.state),
            key,
            chunks: Vec::new(),
            len: 0,
            max_bytes: self.max_bytes,
            chunk_bytes: self.chunk_bytes,
        }))
    }

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        if let Some(removed) = state.artifacts.remove(key) {
            state.used_bytes = state.used_bytes.saturating_sub(removed.len);
        }
        Ok(())
    }

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        Ok(state
            .artifacts
            .iter()
            .filter(|(key, _)| key.namespace() == namespace)
            .map(|(key, artifact)| ArtifactMetadata {
                key: key.clone(),
                length: artifact.len,
            })
            .collect())
    }
}

struct MemoryReadArtifact {
    key: ArtifactKey,
    artifact: MemoryArtifact,
    chunk_bytes: usize,
}

impl ReadArtifact for MemoryReadArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn len(&self) -> Result<u64, RepositoryError> {
        Ok(self.artifact.len)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, RepositoryError> {
        let length = self.artifact.len;
        if offset > length {
            return Err(RepositoryError::OutOfBounds {
                offset,
                end: offset,
                artifact_length: length,
            });
        }
        let available = usize::try_from(length - offset).unwrap_or(usize::MAX);
        let count = available.min(destination.len());
        let mut copied = 0;
        let mut position = usize::try_from(offset).map_err(|_| RepositoryError::OutOfBounds {
            offset,
            end: offset,
            artifact_length: length,
        })?;
        while copied < count {
            let chunk_index = position / self.chunk_bytes;
            let chunk_offset = position % self.chunk_bytes;
            let chunk = self.artifact.chunks.get(chunk_index).ok_or_else(|| {
                RepositoryError::Corrupt("memory artifact has a missing chunk".into())
            })?;
            let chunk_count = (count - copied).min(chunk.len().saturating_sub(chunk_offset));
            if chunk_count == 0 {
                return Err(RepositoryError::Corrupt(
                    "memory artifact chunk is truncated".into(),
                ));
            }
            destination[copied..copied + chunk_count]
                .copy_from_slice(&chunk[chunk_offset..chunk_offset + chunk_count]);
            position += chunk_count;
            copied += chunk_count;
        }
        Ok(count)
    }

    fn region(&self, range: ByteRange) -> Result<Option<ByteRegion>, RepositoryError> {
        if range.end() > self.artifact.len {
            return Err(RepositoryError::OutOfBounds {
                offset: range.offset,
                end: range.end(),
                artifact_length: self.artifact.len,
            });
        }
        if range.length == 0 {
            let backing: Arc<dyn ImmutableByteRegion> = Arc::new(OwnedByteSource::new(
                self.key.identity(),
                Arc::<[u8]>::from([]),
            ));
            return ByteRegion::new(backing, ByteRange::new(0, 0)?)
                .map(Some)
                .map_err(Into::into);
        }
        let start = usize::try_from(range.offset).map_err(|_| RepositoryError::OutOfBounds {
            offset: range.offset,
            end: range.end(),
            artifact_length: self.artifact.len,
        })?;
        let end = usize::try_from(range.end()).map_err(|_| RepositoryError::OutOfBounds {
            offset: range.offset,
            end: range.end(),
            artifact_length: self.artifact.len,
        })?;
        let first_chunk = start / self.chunk_bytes;
        let last_chunk = (end - 1) / self.chunk_bytes;
        if first_chunk != last_chunk {
            return Ok(None);
        }
        let chunk = self.artifact.chunks.get(first_chunk).ok_or_else(|| {
            RepositoryError::Corrupt("memory artifact has a missing chunk".into())
        })?;
        let backing: Arc<dyn ImmutableByteRegion> =
            Arc::new(OwnedByteSource::new(self.key.identity(), Arc::clone(chunk)));
        ByteRegion::new(
            backing,
            ByteRange::new((start % self.chunk_bytes) as u64, range.length)?,
        )
        .map(Some)
        .map_err(Into::into)
    }
}

struct MemoryWriteArtifact {
    repository: Arc<Mutex<MemoryRepositoryState>>,
    key: ArtifactKey,
    chunks: Vec<Vec<u8>>,
    len: u64,
    max_bytes: u64,
    chunk_bytes: usize,
}

impl MemoryWriteArtifact {
    fn resize(&mut self, len: u64) -> Result<(), RepositoryError> {
        if len > self.max_bytes {
            return Err(RepositoryError::QuotaExceeded);
        }
        let chunk_bytes = self.chunk_bytes as u64;
        let chunk_count =
            len.checked_add(chunk_bytes - 1)
                .ok_or(RepositoryError::RangeOverflow {
                    offset: 0,
                    length: len,
                })?
                / chunk_bytes;
        let chunk_count =
            usize::try_from(chunk_count).map_err(|_| RepositoryError::RangeOverflow {
                offset: 0,
                length: len,
            })?;
        self.chunks.resize_with(chunk_count, Vec::new);
        for (index, chunk) in self.chunks.iter_mut().enumerate() {
            let start = (index as u64).saturating_mul(chunk_bytes);
            let chunk_len = (len - start).min(chunk_bytes) as usize;
            chunk.resize(chunk_len, 0);
        }
        self.len = len;
        Ok(())
    }
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
        if self.len < end {
            self.resize(end)?;
        }
        let mut position = usize::try_from(offset).map_err(|_| RepositoryError::RangeOverflow {
            offset,
            length: source_length,
        })?;
        let mut copied = 0;
        while copied < source.len() {
            let chunk_index = position / self.chunk_bytes;
            let chunk_offset = position % self.chunk_bytes;
            let count = (source.len() - copied).min(self.chunk_bytes - chunk_offset);
            self.chunks[chunk_index][chunk_offset..chunk_offset + count]
                .copy_from_slice(&source[copied..copied + count]);
            position += count;
            copied += count;
        }
        Ok(())
    }

    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError> {
        self.resize(len)
    }

    fn flush(&mut self) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn publish(self: Box<Self>) -> Result<(), RepositoryError> {
        let artifact = MemoryArtifact {
            chunks: self
                .chunks
                .into_iter()
                .map(Arc::<[u8]>::from)
                .collect::<Vec<_>>()
                .into(),
            len: self.len,
        };
        let mut state = self
            .repository
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        let previous_len = state
            .artifacts
            .get(&self.key)
            .map_or(0, |previous| previous.len);
        let used_without_previous = state.used_bytes.saturating_sub(previous_len);
        let next_used = used_without_previous
            .checked_add(artifact.len)
            .ok_or(RepositoryError::QuotaExceeded)?;
        if next_used > self.max_bytes {
            return Err(RepositoryError::QuotaExceeded);
        }
        state.artifacts.insert(self.key.clone(), artifact);
        state.used_bytes = next_used;
        Ok(())
    }
}
