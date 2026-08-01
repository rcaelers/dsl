use std::sync::Arc;

use super::artifact::{ArtifactKey, ArtifactRepository, ReadArtifact, RepositoryError};
use super::contract::{
    ByteRange, ImmutableByteRegion, PreparedByteSource, RandomAccessReader, SourceCapabilities,
    SourceIdentity, SourceReadError,
};
use super::memory::OwnedByteSource;

/// Prepared random-access source backed by one immutable repository artifact.
#[derive(Clone)]
pub struct ArtifactByteSource {
    repository: Arc<dyn ArtifactRepository>,
    key: ArtifactKey,
}

impl ArtifactByteSource {
    pub fn new(repository: Arc<dyn ArtifactRepository>, key: ArtifactKey) -> Self {
        Self { repository, key }
    }

    pub fn key(&self) -> &ArtifactKey {
        &self.key
    }
}

impl PreparedByteSource for ArtifactByteSource {
    fn identity(&self) -> SourceIdentity {
        self.key.identity()
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::RANDOM_ACCESS
    }

    fn open_reader(&self) -> Result<Box<dyn RandomAccessReader>, SourceReadError> {
        let reader = self
            .repository
            .open(&self.key)
            .map_err(source_error)?
            .ok_or(SourceReadError::SourceChanged)?;
        Ok(Box::new(ArtifactRandomAccessReader { reader }))
    }
}

/// Returns an immutable artifact range, filling an owned backing when the
/// repository cannot expose its physical backing directly.
pub fn read_artifact_region(
    reader: &mut dyn ReadArtifact,
    range: ByteRange,
) -> Result<Arc<dyn ImmutableByteRegion>, RepositoryError> {
    if let Some(region) = reader.region(range)? {
        return Ok(Arc::new(region));
    }

    let length = usize::try_from(range.length).map_err(|_| RepositoryError::RangeOverflow {
        offset: range.offset,
        length: range.length,
    })?;
    let mut bytes = vec![0_u8; length];
    let mut copied = 0_usize;
    while copied < bytes.len() {
        let offset =
            range
                .offset
                .checked_add(copied as u64)
                .ok_or(RepositoryError::RangeOverflow {
                    offset: range.offset,
                    length: range.length,
                })?;
        let count = reader.read_at(offset, &mut bytes[copied..])?;
        if count == 0 {
            let artifact_length = reader.len()?;
            return Err(RepositoryError::OutOfBounds {
                offset: range.offset,
                end: range.end(),
                artifact_length,
            });
        }
        copied += count;
    }

    let backing: Arc<dyn ImmutableByteRegion> = Arc::new(OwnedByteSource::new(
        reader.key().identity(),
        Arc::<[u8]>::from(bytes),
    ));
    Ok(backing)
}

struct ArtifactRandomAccessReader {
    reader: Box<dyn ReadArtifact>,
}

impl RandomAccessReader for ArtifactRandomAccessReader {
    fn len(&self) -> Result<u64, SourceReadError> {
        self.reader.len().map_err(source_error)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, SourceReadError> {
        self.reader
            .read_at(offset, destination)
            .map_err(source_error)
    }
}

fn source_error(error: RepositoryError) -> SourceReadError {
    match error {
        RepositoryError::RangeOverflow { offset, length } => {
            SourceReadError::RangeOverflow { offset, length }
        }
        RepositoryError::OutOfBounds {
            offset,
            end,
            artifact_length,
        } => SourceReadError::OutOfBounds {
            offset,
            end,
            source_length: artifact_length,
        },
        RepositoryError::Corrupt(_) => SourceReadError::SourceChanged,
        error => SourceReadError::Io(error.to_string()),
    }
}
