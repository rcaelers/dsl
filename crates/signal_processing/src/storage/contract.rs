/// Stable content-oriented identity used to address a prepared byte source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceIdentity([u8; 32]);

impl SourceIdentity {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A fixed-width byte range independent of the host address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

impl ByteRange {
    pub fn new(offset: u64, length: u64) -> Result<Self, SourceReadError> {
        offset
            .checked_add(length)
            .ok_or(SourceReadError::RangeOverflow { offset, length })?;
        Ok(Self { offset, length })
    }

    pub fn end(self) -> u64 {
        self.offset + self.length
    }
}

/// Access properties used by planners without inspecting the source kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceCapabilities {
    pub independent_readers: bool,
    pub efficient_random_access: bool,
}

impl SourceCapabilities {
    pub const RANDOM_ACCESS: Self = Self {
        independent_readers: true,
        efficient_random_access: true,
    };
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceReadError {
    #[error("byte range {offset}..+{length} overflows the source address space")]
    RangeOverflow { offset: u64, length: u64 },
    #[error("byte range {offset}..{end} exceeds source length {source_length}")]
    OutOfBounds {
        offset: u64,
        end: u64,
        source_length: u64,
    },
    #[error("the prepared byte source changed while it was being read")]
    SourceChanged,
    #[error("byte source I/O failed: {0}")]
    Io(String),
}

/// One cursor-independent read session for a prepared source.
pub trait RandomAccessReader: Send {
    fn len(&self) -> Result<u64, SourceReadError>;

    fn is_empty(&self) -> Result<bool, SourceReadError> {
        self.len().map(|length| length == 0)
    }

    /// Reads from `offset`, returning the number of bytes copied. A short read
    /// is valid only at the end of the source.
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, SourceReadError>;

    fn read_exact_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> Result<(), SourceReadError> {
        let requested =
            u64::try_from(destination.len()).map_err(|_| SourceReadError::RangeOverflow {
                offset,
                length: u64::MAX,
            })?;
        let range = ByteRange::new(offset, requested)?;
        let source_length = self.len()?;
        if range.end() > source_length {
            return Err(SourceReadError::OutOfBounds {
                offset,
                end: range.end(),
                source_length,
            });
        }

        let mut copied = 0_usize;
        while copied < destination.len() {
            let read_offset = offset + copied as u64;
            let count = self.read_at(read_offset, &mut destination[copied..])?;
            if count == 0 {
                return Err(SourceReadError::Io(format!(
                    "source returned no data at offset {read_offset}"
                )));
            }
            copied += count;
        }
        Ok(())
    }
}

/// A host-acquired source ready for shared parsing and indexing algorithms.
pub trait PreparedByteSource: Send + Sync {
    fn identity(&self) -> SourceIdentity;

    fn capabilities(&self) -> SourceCapabilities;

    fn open_reader(&self) -> Result<Box<dyn RandomAccessReader>, SourceReadError>;
}

/// Immutable bytes that may be owned memory, mmap, or another host backing.
pub trait ImmutableByteRegion: Send + Sync {
    fn bytes(&self) -> &[u8];

    fn len(&self) -> u64 {
        self.bytes().len() as u64
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn slice(&self, range: ByteRange) -> Result<&[u8], SourceReadError> {
        let source_length = self.len();
        if range.end() > source_length {
            return Err(SourceReadError::OutOfBounds {
                offset: range.offset,
                end: range.end(),
                source_length,
            });
        }
        let start = range.offset as usize;
        let end = range.end() as usize;
        Ok(&self.bytes()[start..end])
    }
}
