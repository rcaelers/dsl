use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::{
    ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactRepository, ReadArtifact,
    RepositoryCapabilities, RepositoryError, SourceIdentity, WriteArtifact,
};

/// One bounded repository mutation transferred from an execution host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactReplicationEvent {
    PublishedChunk {
        namespace: String,
        identity: SourceIdentity,
        offset: u64,
        total_length: u64,
        data: Vec<u8>,
        complete: bool,
    },
    Removed {
        namespace: String,
        identity: SourceIdentity,
    },
}

struct PendingPublication {
    key: ArtifactKey,
    reader: Box<dyn ReadArtifact>,
    length: u64,
    offset: u64,
}

enum PendingMutation {
    Published(PendingPublication),
    Removed(ArtifactKey),
}

/// Repository decorator that journals immutable publications for bounded transfer.
#[derive(Clone)]
pub struct ReplicatingArtifactRepository {
    inner: Arc<dyn ArtifactRepository>,
    pending: Arc<Mutex<VecDeque<PendingMutation>>>,
}

impl ReplicatingArtifactRepository {
    pub fn new(inner: Arc<dyn ArtifactRepository>) -> Self {
        Self {
            inner,
            pending: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending
            .lock()
            .map(|pending| !pending.is_empty())
            .unwrap_or(true)
    }

    /// Drops journal entries without changing the wrapped repository.
    pub fn discard_pending(&self) -> Result<(), RepositoryError> {
        self.pending
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?
            .clear();
        Ok(())
    }

    /// Drains bounded mutation events while retaining immutable generations until transferred.
    pub fn drain(
        &self,
        max_events: usize,
        max_payload_bytes: usize,
    ) -> Result<Vec<ArtifactReplicationEvent>, RepositoryError> {
        if max_events == 0 || max_payload_bytes == 0 {
            return Err(RepositoryError::Unsupported(
                "artifact replication limits must be non-zero".to_owned(),
            ));
        }
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        let mut events = Vec::new();
        let mut payload_bytes = 0_usize;
        while events.len() < max_events {
            let Some(mutation) = pending.front_mut() else {
                break;
            };
            match mutation {
                PendingMutation::Removed(key) => {
                    events.push(ArtifactReplicationEvent::Removed {
                        namespace: key.namespace().as_str().to_owned(),
                        identity: key.identity(),
                    });
                    pending.pop_front();
                }
                PendingMutation::Published(publication) => {
                    let remaining_budget = max_payload_bytes.saturating_sub(payload_bytes);
                    if remaining_budget == 0 && !events.is_empty() {
                        break;
                    }
                    let remaining = publication.length.saturating_sub(publication.offset);
                    let count = usize::try_from(remaining)
                        .unwrap_or(usize::MAX)
                        .min(remaining_budget.max(1));
                    let mut data = vec![0_u8; count];
                    read_exact_at(publication.reader.as_mut(), publication.offset, &mut data)?;
                    let offset = publication.offset;
                    publication.offset = publication.offset.saturating_add(count as u64);
                    let complete = publication.offset == publication.length;
                    events.push(ArtifactReplicationEvent::PublishedChunk {
                        namespace: publication.key.namespace().as_str().to_owned(),
                        identity: publication.key.identity(),
                        offset,
                        total_length: publication.length,
                        data,
                        complete,
                    });
                    payload_bytes = payload_bytes.saturating_add(count);
                    if complete {
                        pending.pop_front();
                    }
                }
            }
        }
        Ok(events)
    }
}

impl ArtifactRepository for ReplicatingArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities {
        self.inner.capabilities()
    }

    fn namespaces(&self) -> Result<Vec<ArtifactNamespace>, RepositoryError> {
        self.inner.namespaces()
    }

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError> {
        self.inner.open(key)
    }

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError> {
        let writer = self.inner.begin_write(key.clone())?;
        Ok(Box::new(ReplicatingWriteArtifact {
            repository: Arc::clone(&self.inner),
            pending: Arc::clone(&self.pending),
            inner: writer,
            key,
            length: 0,
        }))
    }

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError> {
        self.inner.remove(key)?;
        self.pending
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?
            .push_back(PendingMutation::Removed(key.clone()));
        Ok(())
    }

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError> {
        self.inner.entries(namespace)
    }
}

struct ReplicatingWriteArtifact {
    repository: Arc<dyn ArtifactRepository>,
    pending: Arc<Mutex<VecDeque<PendingMutation>>>,
    inner: Box<dyn WriteArtifact>,
    key: ArtifactKey,
    length: u64,
}

impl WriteArtifact for ReplicatingWriteArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError> {
        self.inner.write_at(offset, source)?;
        let source_length =
            u64::try_from(source.len()).map_err(|_| RepositoryError::RangeOverflow {
                offset,
                length: u64::MAX,
            })?;
        self.length = self.length.max(offset.checked_add(source_length).ok_or(
            RepositoryError::RangeOverflow {
                offset,
                length: source_length,
            },
        )?);
        Ok(())
    }

    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError> {
        self.inner.truncate(len)?;
        self.length = len;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), RepositoryError> {
        self.inner.flush()
    }

    fn publish(self: Box<Self>) -> Result<(), RepositoryError> {
        let Self {
            repository,
            pending,
            inner,
            key,
            length,
        } = *self;
        inner.publish()?;
        let reader = repository.open(&key)?.ok_or_else(|| {
            RepositoryError::Corrupt("published artifact is unavailable for replication".to_owned())
        })?;
        pending
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?
            .push_back(PendingMutation::Published(PendingPublication {
                key,
                reader,
                length,
                offset: 0,
            }));
        Ok(())
    }
}

struct PendingReceive {
    writer: Box<dyn WriteArtifact>,
    next_offset: u64,
    total_length: u64,
}

/// Applies bounded replication events atomically to a destination repository.
pub struct ArtifactReplicationReceiver {
    repository: Arc<dyn ArtifactRepository>,
    pending: BTreeMap<ArtifactKey, PendingReceive>,
}

impl ArtifactReplicationReceiver {
    pub fn new(repository: Arc<dyn ArtifactRepository>) -> Self {
        Self {
            repository,
            pending: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, event: ArtifactReplicationEvent) -> Result<(), RepositoryError> {
        match event {
            ArtifactReplicationEvent::Removed {
                namespace,
                identity,
            } => {
                let key = artifact_key(namespace, identity)?;
                self.pending.remove(&key);
                self.repository.remove(&key)
            }
            ArtifactReplicationEvent::PublishedChunk {
                namespace,
                identity,
                offset,
                total_length,
                data,
                complete,
            } => {
                let key = artifact_key(namespace, identity)?;
                if offset == 0 {
                    self.pending.insert(
                        key.clone(),
                        PendingReceive {
                            writer: self.repository.begin_write(key.clone())?,
                            next_offset: 0,
                            total_length,
                        },
                    );
                }
                let pending = self.pending.get_mut(&key).ok_or_else(|| {
                    RepositoryError::Corrupt(
                        "artifact replication chunk has no active publication".to_owned(),
                    )
                })?;
                if pending.next_offset != offset || pending.total_length != total_length {
                    return Err(RepositoryError::Corrupt(
                        "artifact replication chunks are out of order".to_owned(),
                    ));
                }
                pending.writer.write_at(offset, &data)?;
                pending.next_offset = pending.next_offset.saturating_add(data.len() as u64);
                if complete {
                    if pending.next_offset != pending.total_length {
                        return Err(RepositoryError::Corrupt(
                            "artifact replication completed at the wrong length".to_owned(),
                        ));
                    }
                    let mut pending = self.pending.remove(&key).expect("entry exists above");
                    pending.writer.truncate(total_length)?;
                    pending.writer.flush()?;
                    pending.writer.publish()?;
                }
                Ok(())
            }
        }
    }

    pub fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }
}

fn artifact_key(
    namespace: String,
    identity: SourceIdentity,
) -> Result<ArtifactKey, RepositoryError> {
    Ok(ArtifactKey::new(
        ArtifactNamespace::new(namespace)?,
        identity,
    ))
}

fn read_exact_at(
    reader: &mut dyn ReadArtifact,
    offset: u64,
    destination: &mut [u8],
) -> Result<(), RepositoryError> {
    let mut completed = 0;
    while completed < destination.len() {
        let count = reader.read_at(offset + completed as u64, &mut destination[completed..])?;
        if count == 0 {
            return Err(RepositoryError::Corrupt(
                "published artifact ended during replication".to_owned(),
            ));
        }
        completed += count;
    }
    Ok(())
}

#[cfg(test)]
mod replication_tests {
    use super::*;
    use crate::MemoryArtifactRepository;

    fn key() -> ArtifactKey {
        ArtifactKey::new(
            ArtifactNamespace::new("replication-test").unwrap(),
            SourceIdentity::from_bytes([41; 32]),
        )
    }

    #[test]
    fn publications_cross_repositories_in_bounded_chunks() {
        let source: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let source = ReplicatingArtifactRepository::new(source);
        let destination: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let mut receiver = ArtifactReplicationReceiver::new(Arc::clone(&destination));
        let mut writer = source.begin_write(key()).unwrap();
        writer
            .write_at(0, b"bounded repository replication")
            .unwrap();
        writer.publish().unwrap();

        let first = source.drain(1, 7).unwrap();
        assert_eq!(first.len(), 1);
        assert!(matches!(
            &first[0],
            ArtifactReplicationEvent::PublishedChunk {
                offset: 0,
                data,
                complete: false,
                ..
            } if data.len() == 7
        ));
        receiver.apply(first.into_iter().next().unwrap()).unwrap();
        while source.has_pending() {
            for event in source.drain(2, 7).unwrap() {
                receiver.apply(event).unwrap();
            }
        }

        assert!(receiver.is_idle());
        let mut reader = destination.open(&key()).unwrap().unwrap();
        let mut bytes = vec![0; reader.len().unwrap() as usize];
        read_exact_at(reader.as_mut(), 0, &mut bytes).unwrap();
        assert_eq!(bytes, b"bounded repository replication");
    }

    #[test]
    fn removal_is_replicated_after_publication() {
        let source: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let source = ReplicatingArtifactRepository::new(source);
        let destination: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let mut receiver = ArtifactReplicationReceiver::new(Arc::clone(&destination));
        let mut writer = source.begin_write(key()).unwrap();
        writer.write_at(0, b"temporary").unwrap();
        writer.publish().unwrap();
        source.remove(&key()).unwrap();

        while source.has_pending() {
            for event in source.drain(4, 64).unwrap() {
                receiver.apply(event).unwrap();
            }
        }

        assert!(destination.open(&key()).unwrap().is_none());
    }
}
