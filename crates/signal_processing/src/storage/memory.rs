use std::sync::Arc;

use super::contract::{
    ImmutableByteRegion, PreparedByteSource, RandomAccessReader, SourceCapabilities,
    SourceIdentity, SourceReadError,
};

/// Portable prepared source backed by immutable owned memory.
#[derive(Clone)]
pub struct OwnedByteSource {
    identity: SourceIdentity,
    bytes: Arc<[u8]>,
}

impl OwnedByteSource {
    pub fn new(identity: SourceIdentity, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            identity,
            bytes: bytes.into(),
        }
    }
}

impl PreparedByteSource for OwnedByteSource {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::RANDOM_ACCESS
    }

    fn open_reader(&self) -> Result<Box<dyn RandomAccessReader>, SourceReadError> {
        Ok(Box::new(OwnedByteReader {
            bytes: Arc::clone(&self.bytes),
        }))
    }
}

impl ImmutableByteRegion for OwnedByteSource {
    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

struct OwnedByteReader {
    bytes: Arc<[u8]>,
}

impl RandomAccessReader for OwnedByteReader {
    fn len(&self) -> Result<u64, SourceReadError> {
        Ok(self.bytes.len() as u64)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, SourceReadError> {
        let source_length = self.bytes.len() as u64;
        if offset > source_length {
            return Err(SourceReadError::OutOfBounds {
                offset,
                end: offset,
                source_length,
            });
        }
        let start = offset as usize;
        let count = destination.len().min(self.bytes.len() - start);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}
