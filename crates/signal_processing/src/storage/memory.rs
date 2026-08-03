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

/// Portable prepared source backed by independently owned immutable chunks.
///
/// This avoids retaining one address-space-sized allocation for imported browser files while
/// preserving the same random-access contract used by native file and mmap adapters.
#[derive(Clone)]
pub struct ChunkedByteSource {
    identity: SourceIdentity,
    chunks: Arc<[Arc<[u8]>]>,
    chunk_size: usize,
    length: u64,
}

impl ChunkedByteSource {
    /// Validates and creates an immutable random-access source from fixed-size chunks.
    ///
    /// # Parameters
    /// - `identity`: Stable content identity represented by the chunks.
    /// - `chunks`: Non-empty immutable chunks, except that the complete sequence may be empty.
    /// - `chunk_size`: Required size of each non-final chunk.
    pub fn new(
        identity: SourceIdentity,
        chunks: Vec<Arc<[u8]>>,
        chunk_size: usize,
    ) -> Result<Self, SourceReadError> {
        if chunk_size == 0 {
            return Err(SourceReadError::Io(
                "chunked byte sources require a non-zero chunk size".to_owned(),
            ));
        }
        for (index, chunk) in chunks.iter().enumerate() {
            let final_chunk = index + 1 == chunks.len();
            if chunk.is_empty()
                || chunk.len() > chunk_size
                || (!final_chunk && chunk.len() != chunk_size)
            {
                return Err(SourceReadError::Io(
                    "chunked byte source contains a malformed chunk sequence".to_owned(),
                ));
            }
        }
        let length = chunks.iter().try_fold(0_u64, |total, chunk| {
            let chunk_length =
                u64::try_from(chunk.len()).map_err(|_| SourceReadError::RangeOverflow {
                    offset: total,
                    length: u64::MAX,
                })?;
            total
                .checked_add(chunk_length)
                .ok_or(SourceReadError::RangeOverflow {
                    offset: total,
                    length: chunk_length,
                })
        })?;
        Ok(Self {
            identity,
            chunks: chunks.into(),
            chunk_size,
            length,
        })
    }
}

impl PreparedByteSource for ChunkedByteSource {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::RANDOM_ACCESS
    }

    fn open_reader(&self) -> Result<Box<dyn RandomAccessReader>, SourceReadError> {
        Ok(Box::new(ChunkedByteReader {
            chunks: Arc::clone(&self.chunks),
            chunk_size: self.chunk_size,
            length: self.length,
        }))
    }
}

struct ChunkedByteReader {
    chunks: Arc<[Arc<[u8]>]>,
    chunk_size: usize,
    length: u64,
}

impl RandomAccessReader for ChunkedByteReader {
    fn len(&self) -> Result<u64, SourceReadError> {
        Ok(self.length)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, SourceReadError> {
        if offset > self.length {
            return Err(SourceReadError::OutOfBounds {
                offset,
                end: offset,
                source_length: self.length,
            });
        }
        let mut source_offset =
            usize::try_from(offset).map_err(|_| SourceReadError::RangeOverflow {
                offset,
                length: u64::try_from(destination.len()).unwrap_or(u64::MAX),
            })?;
        let available = usize::try_from(self.length - offset).unwrap_or(usize::MAX);
        let requested = destination.len().min(available);
        let mut copied = 0_usize;
        while copied < requested {
            let chunk_index = source_offset / self.chunk_size;
            let chunk_offset = source_offset % self.chunk_size;
            let chunk = &self.chunks[chunk_index];
            let count = (requested - copied).min(chunk.len() - chunk_offset);
            destination[copied..copied + count]
                .copy_from_slice(&chunk[chunk_offset..chunk_offset + count]);
            copied += count;
            source_offset += count;
        }
        Ok(copied)
    }
}

impl OwnedByteSource {
    /// Creates an immutable random-access source from one owned byte allocation.
    ///
    /// # Parameters
    /// - `identity`: Stable content identity represented by the bytes.
    /// - `bytes`: Immutable byte allocation to retain.
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
        u64::try_from(self.bytes.len()).map_err(|_| SourceReadError::RangeOverflow {
            offset: 0,
            length: u64::MAX,
        })
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, SourceReadError> {
        let source_length =
            u64::try_from(self.bytes.len()).map_err(|_| SourceReadError::RangeOverflow {
                offset: 0,
                length: u64::MAX,
            })?;
        if offset > source_length {
            return Err(SourceReadError::OutOfBounds {
                offset,
                end: offset,
                source_length,
            });
        }
        let start = usize::try_from(offset)
            .map_err(|_| SourceReadError::RangeOverflow { offset, length: 0 })?;
        let count = destination.len().min(self.bytes.len() - start);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn chunked_source_reads_across_boundaries_with_independent_readers() {
        let source = ChunkedByteSource::new(
            SourceIdentity::from_bytes([0x42; 32]),
            vec![
                Arc::from(&b"abcd"[..]),
                Arc::from(&b"efgh"[..]),
                Arc::from(&b"ij"[..]),
            ],
            4,
        )
        .unwrap();
        let mut first = source.open_reader().unwrap();
        let mut second = source.open_reader().unwrap();
        let mut across = [0_u8; 7];
        let mut tail = [0_u8; 2];

        first.read_exact_at(2, &mut across).unwrap();
        second.read_exact_at(8, &mut tail).unwrap();

        assert_eq!(&across, b"cdefghi");
        assert_eq!(&tail, b"ij");
    }

    #[test]
    fn chunked_source_rejects_malformed_nonfinal_chunks() {
        assert!(matches!(
            ChunkedByteSource::new(
                SourceIdentity::from_bytes([0x43; 32]),
                vec![Arc::from(&b"abc"[..]), Arc::from(&b"defg"[..])],
                4,
            ),
            Err(SourceReadError::Io(_))
        ));
        assert!(matches!(
            ChunkedByteSource::new(
                SourceIdentity::from_bytes([0x44; 32]),
                vec![Arc::from(&b"abcde"[..])],
                4,
            ),
            Err(SourceReadError::Io(_))
        ));
    }
}
