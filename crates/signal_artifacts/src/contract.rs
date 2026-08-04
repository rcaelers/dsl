use serde::{Deserialize, Serialize};

/// Stable content-oriented identity used to address a prepared byte source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceIdentity([u8; 32]);

impl SourceIdentity {
    /// Creates an identity from a content digest or equivalent stable bytes.
    ///
    /// # Parameters
    /// - `bytes`: Exact 32-byte identity value.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the exact stable identity bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A fixed-width byte range independent of the host address space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    /// Starting byte offset.
    pub offset: u64,
    /// Number of bytes in the range.
    pub length: u64,
}

impl ByteRange {
    /// Creates a range after checking that its exclusive end fits in `u64`.
    pub fn new(offset: u64, length: u64) -> Result<Self, SourceReadError> {
        offset
            .checked_add(length)
            .ok_or(SourceReadError::RangeOverflow { offset, length })?;
        Ok(Self { offset, length })
    }

    /// Returns the exclusive end offset.
    pub fn end(self) -> u64 {
        self.offset + self.length
    }
}

/// Access properties used by planners without inspecting the source kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceCapabilities {
    /// Whether separately opened readers may advance independently.
    pub independent_readers: bool,
    /// Whether the source supports efficient arbitrary-range reads.
    pub efficient_random_access: bool,
}

impl SourceCapabilities {
    /// Capabilities of a source with independent efficient random access.
    pub const RANDOM_ACCESS: Self = Self {
        independent_readers: true,
        efficient_random_access: true,
    };
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceReadError {
    #[error("byte range {offset}..+{length} overflows the source address space")]
    /// Requested range arithmetic overflowed the source address space.
    RangeOverflow {
        /// Starting byte offset.
        offset: u64,
        /// Requested byte count.
        length: u64,
    },
    #[error("byte range {offset}..{end} exceeds source length {source_length}")]
    /// Requested range extends beyond the source length.
    OutOfBounds {
        /// Starting byte offset.
        offset: u64,
        /// First byte after the requested range.
        end: u64,
        /// Current source length.
        source_length: u64,
    },
    #[error("the prepared byte source changed while it was being read")]
    SourceChanged,
    #[error("byte source I/O failed: {0}")]
    Io(String),
}

/// One cursor-independent read session for a prepared source.
pub trait RandomAccessReader: Send {
    /// Returns the current byte length visible to this reader.
    fn len(&self) -> Result<u64, SourceReadError>;

    /// Returns whether the source contains no bytes.
    fn is_empty(&self) -> Result<bool, SourceReadError> {
        self.len().map(|length| length == 0)
    }

    /// Reads from `offset`, returning the number of bytes copied. A short read
    /// is valid only at the end of the source.
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, SourceReadError>;

    /// Reads exactly the destination length or returns an error.
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
    /// Returns a stable content identity for this source generation.
    fn identity(&self) -> SourceIdentity;

    /// Returns source access capabilities.
    fn capabilities(&self) -> SourceCapabilities;

    /// Opens an independent random-access reader.
    fn open_reader(&self) -> Result<Box<dyn RandomAccessReader>, SourceReadError>;
}

/// Immutable bytes that may be owned memory, mmap, or another host backing.
pub trait ImmutableByteRegion: Send + Sync {
    /// Returns immutable bytes held by this backing.
    fn bytes(&self) -> &[u8];

    /// Returns byte length of the immutable backing.
    fn len(&self) -> u64 {
        u64::try_from(self.bytes().len()).expect("resident byte regions fit in u64")
    }

    /// Returns whether the immutable backing contains no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a validated subrange of the immutable backing.
    ///
    /// # Parameters
    /// - `range`: Byte range to expose from this backing.
    fn slice(&self, range: ByteRange) -> Result<&[u8], SourceReadError> {
        let source_length = self.len();
        if range.end() > source_length {
            return Err(SourceReadError::OutOfBounds {
                offset: range.offset,
                end: range.end(),
                source_length,
            });
        }
        let start = usize::try_from(range.offset).map_err(|_| SourceReadError::RangeOverflow {
            offset: range.offset,
            length: range.length,
        })?;
        let end = usize::try_from(range.end()).map_err(|_| SourceReadError::RangeOverflow {
            offset: range.offset,
            length: range.length,
        })?;
        Ok(&self.bytes()[start..end])
    }
}

/// A fixed range of an immutable backing that keeps that backing alive.
#[derive(Clone)]
pub struct ByteRegion {
    backing: Arc<dyn ImmutableByteRegion>,
    range: ByteRange,
}

impl ByteRegion {
    /// Creates a validated shared view of an immutable byte range.
    pub fn new(
        backing: Arc<dyn ImmutableByteRegion>,
        range: ByteRange,
    ) -> Result<Self, SourceReadError> {
        backing.slice(range)?;
        Ok(Self { backing, range })
    }

    /// Returns the absolute byte range visible through this view.
    pub fn range(&self) -> ByteRange {
        self.range
    }

    /// Returns the visible immutable bytes without copying.
    pub fn bytes(&self) -> &[u8] {
        self.backing
            .slice(self.range)
            .expect("a byte region validates its immutable backing at construction")
    }

    /// Clones the immutable backing so another validated region can share it.
    pub fn clone_backing(&self) -> Arc<dyn ImmutableByteRegion> {
        Arc::clone(&self.backing)
    }

    /// Returns whether two views retain the same immutable backing allocation.
    pub fn shares_backing(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.backing, &other.backing)
            || (!self.backing.is_empty()
                && self.backing.bytes().as_ptr() == other.backing.bytes().as_ptr()
                && self.backing.len() == other.backing.len())
    }
}

impl ImmutableByteRegion for ByteRegion {
    fn bytes(&self) -> &[u8] {
        Self::bytes(self)
    }
}
use std::sync::Arc;
