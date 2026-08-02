//! Repository-backed finite-capture waveform index.
//!
//! Each `(channel, block)` leaf is an immutable artifact containing its exact
//! summary hierarchy. A compact root artifact contains the capture metadata
//! and leaf directory and is published last, so readers observe either the
//! previous complete generation or the new complete generation. Artifact
//! readers retain either mmap-backed or owned-memory byte regions according
//! to the injected repository; the index never requires one capture-sized
//! allocation.

use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use super::types::{
    BlockIndex, DIR_ENTRY_SIZE, HEADER_SIZE, IndexHeader, L1_WORDS, L2_WORDS, MAGIC, RootDirEntry,
};
use crate::capture::CaptureMetadata;
use crate::{
    ArtifactKey, ArtifactNamespace, ArtifactRepository, Error, RepositoryError, Result,
    SourceIdentity,
};

// ---------------------------------------------------------------------------
// IndexWriter — create and publish a new index generation
// ---------------------------------------------------------------------------

/// Publishes a new index generation for one capture source.
///
/// Leaf artifacts may complete in any order. [`IndexWriter::finish`] publishes
/// the root artifact only after all leaves are available.
pub(crate) struct IndexWriter {
    repository: Arc<dyn ArtifactRepository>,
    identity: SourceIdentity,
    directory: Vec<Vec<RootDirEntry>>,
    index_header: IndexHeader,
}

impl IndexWriter {
    /// Starts an unpublished index generation.
    pub(crate) fn create(
        repository: Arc<dyn ArtifactRepository>,
        identity: SourceIdentity,
        capture_header: &CaptureMetadata,
        source_revision: u64,
    ) -> Result<Self> {
        let channels = capture_header.total_probes;
        let total_blocks = usize::try_from(capture_header.total_blocks).map_err(|_| {
            Error::ParseError("waveform-index block count exceeds this address space".into())
        })?;
        let directory_entries = channels.checked_mul(total_blocks).ok_or_else(|| {
            Error::ParseError("waveform-index directory entry count overflows usize".into())
        })?;
        let dir_offset = HEADER_SIZE;
        let payload_offset = u64::try_from(directory_entries)
            .ok()
            .and_then(|entries| entries.checked_mul(DIR_ENTRY_SIZE))
            .and_then(|directory_bytes| dir_offset.checked_add(directory_bytes))
            .ok_or_else(|| Error::ParseError("waveform-index directory size overflow".into()))?;

        let index_header = IndexHeader {
            source_revision,
            total_samples: capture_header.total_samples,
            total_blocks: capture_header.total_blocks,
            samples_per_block: capture_header.samples_per_block,
            samplerate_bits: capture_header.samplerate_hz.to_bits(),
            total_channels: u64::try_from(channels).map_err(|_| {
                Error::ParseError("waveform-index channel count exceeds u64".into())
            })?,
            blocks_per_channel: capture_header.total_blocks,
            dir_offset,
            payload_offset,
        };

        Ok(Self {
            repository,
            identity,
            directory: vec![vec![RootDirEntry::default(); total_blocks]; channels],
            index_header,
        })
    }

    /// Serialize `leaf` and append its chunk to the payload; record the directory entry.
    pub(crate) fn write_block(
        &mut self,
        channel: usize,
        block: usize,
        leaf: &BlockIndex,
    ) -> Result<()> {
        let payload = serialize_leaf(leaf);
        publish(
            self.repository.as_ref(),
            leaf_key(self.identity, channel, block)?,
            &payload,
        )?;
        self.directory[channel][block] = RootDirEntry {
            offset: 0,
            len: payload.len() as u64,
            toggle: leaf.levels.is_some(),
            first: leaf.first,
            last: leaf.last,
            l3_toggle: leaf.levels.as_ref().map_or(0, |l| l.l3_toggle),
            l3_last: leaf.levels.as_ref().map_or(0, |l| l.l3_last),
        };
        Ok(())
    }

    /// Publishes the root and makes the completed generation discoverable.
    pub(crate) fn finish(self) -> Result<()> {
        let root_capacity = usize::try_from(self.index_header.payload_offset).map_err(|_| {
            Error::ParseError("waveform-index root exceeds this address space".into())
        })?;
        let mut bytes = Vec::with_capacity(root_capacity);
        Self::write_header(&mut bytes, &self.index_header)?;
        for channel_dir in &self.directory {
            for entry in channel_dir {
                Self::write_dir_entry(&mut bytes, entry)?;
            }
        }
        publish(self.repository.as_ref(), root_key(self.identity)?, &bytes)
    }

    fn write_header(file: &mut impl Write, header: &IndexHeader) -> Result<()> {
        file.write_all(MAGIC)?;
        write_u32(file, 7)?;
        write_u32(file, HEADER_SIZE as u32)?;
        write_u64(file, header.source_revision)?;
        write_u64(file, header.total_samples)?;
        write_u64(file, header.total_blocks)?;
        write_u64(file, header.samples_per_block)?;
        write_u64(file, header.samplerate_bits)?;
        write_u64(file, header.total_channels)?;
        write_u64(file, header.blocks_per_channel)?;
        write_u64(file, header.dir_offset)?;
        write_u64(file, header.payload_offset)?;
        let written = 8 + 4 + 4 + 8 * 9;
        file.write_all(&vec![0_u8; HEADER_SIZE as usize - written])?;
        Ok(())
    }

    fn write_dir_entry(file: &mut impl Write, entry: &RootDirEntry) -> Result<()> {
        debug_assert_eq!(DIR_ENTRY_SIZE, 40);
        write_u64(file, entry.offset)?;
        write_u64(file, entry.len)?;
        let flags = (entry.toggle as u8) | ((entry.first as u8) << 1) | ((entry.last as u8) << 2);
        file.write_all(&[flags, 0, 0, 0, 0, 0, 0, 0])?;
        write_u64(file, entry.l3_toggle)?;
        write_u64(file, entry.l3_last)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// IndexReader — read an existing index generation
// ---------------------------------------------------------------------------

const LEAF_CACHE_CAPACITY: usize = 128;

/// Decoded view of one leaf artifact.
#[derive(Clone)]
pub(crate) struct LeafView {
    pub(crate) valid_samples: u32,
    pub(crate) first: bool,
    pub(crate) last: bool,
    pub(crate) levels: Option<LevelsView>,
}

#[derive(Clone)]
pub(crate) struct LevelsView {
    pub l1_toggle: Arc<[u64]>,
    pub l1_last: Arc<[u64]>,
    pub l2_toggle: Arc<[u64]>,
    pub l2_last: Arc<[u64]>,
    pub l3_toggle: u64,
}

pub(crate) struct IndexReader {
    repository: Arc<dyn ArtifactRepository>,
    identity: SourceIdentity,
    header: CaptureMetadata,
    directory: Vec<Vec<RootDirEntry>>,
    leaf_cache: Mutex<LeafCache>,
}

#[derive(Default)]
struct LeafCache {
    entries: HashMap<(usize, usize), LeafView>,
    recency: VecDeque<(usize, usize)>,
}

impl LeafCache {
    fn get(&mut self, key: (usize, usize)) -> Option<LeafView> {
        let value = self.entries.get(&key)?.clone();
        self.touch(key);
        Some(value)
    }

    fn insert(&mut self, key: (usize, usize), value: LeafView) {
        self.entries.insert(key, value);
        self.touch(key);
        while self.entries.len() > LEAF_CACHE_CAPACITY {
            if let Some(oldest) = self.recency.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    fn touch(&mut self, key: (usize, usize)) {
        if let Some(index) = self.recency.iter().position(|candidate| *candidate == key) {
            self.recency.remove(index);
        }
        self.recency.push_back(key);
    }
}

impl IndexReader {
    pub(crate) fn is_valid(
        repository: &dyn ArtifactRepository,
        identity: SourceIdentity,
        header: &CaptureMetadata,
        source_revision: u64,
    ) -> Result<bool> {
        let Ok(Some(bytes)) = read(repository, &root_key(identity)?) else {
            return Ok(false);
        };
        let mut file = Cursor::new(bytes);
        let Ok(index_header) = Self::read_header(&mut file) else {
            return Ok(false);
        };
        Ok(Self::validate_header(&index_header, header, source_revision).is_ok())
    }

    pub(crate) fn open(
        repository: Arc<dyn ArtifactRepository>,
        identity: SourceIdentity,
        header: CaptureMetadata,
        source_revision: u64,
    ) -> Result<Self> {
        let bytes = read(repository.as_ref(), &root_key(identity)?)?
            .ok_or_else(|| Error::ParseError("waveform index is missing".into()))?;
        let mut file = Cursor::new(bytes);
        let index_header = Self::read_header(&mut file)?;
        Self::validate_header(&index_header, &header, source_revision)?;
        let blocks_per_channel =
            usize::try_from(index_header.blocks_per_channel).map_err(|_| {
                Error::ParseError("waveform-index block count exceeds this address space".into())
            })?;
        let directory = Self::read_directory(
            &mut file,
            &index_header,
            header.total_probes,
            blocks_per_channel,
        )?;
        Ok(Self {
            repository,
            identity,
            header,
            directory,
            leaf_cache: Mutex::new(LeafCache::default()),
        })
    }

    pub(crate) fn identity(&self) -> SourceIdentity {
        self.identity
    }

    pub(crate) fn header(&self) -> &CaptureMetadata {
        &self.header
    }

    pub(crate) fn load_leaf(&self, channel: usize, block: usize) -> Result<LeafView> {
        let key = (channel, block);
        if let Some(leaf) = self
            .leaf_cache
            .lock()
            .map_err(|_| Error::ParseError("waveform-index leaf cache is unavailable".into()))?
            .get(key)
        {
            return Ok(leaf);
        }

        let entry = self
            .directory
            .get(channel)
            .and_then(|blocks| blocks.get(block))
            .copied()
            .ok_or_else(|| Error::ParseError("block index out of bounds".to_string()))?;
        let data = read(
            self.repository.as_ref(),
            &leaf_key(self.identity, channel, block)?,
        )?
        .ok_or_else(|| Error::ParseError("waveform index leaf is missing".into()))?;
        if data.len() as u64 != entry.len {
            return Err(Error::ParseError("waveform index leaf is truncated".into()));
        }
        let leaf = leaf_view(&data)?;
        let block_start = block as u64 * self.header.samples_per_block;
        let expected_samples = self
            .header
            .total_samples
            .saturating_sub(block_start)
            .min(self.header.samples_per_block)
            .min(u32::MAX as u64) as u32;
        if leaf.valid_samples != expected_samples {
            return Err(Error::ParseError(
                "invalid waveform-index leaf length".to_string(),
            ));
        }
        self.leaf_cache
            .lock()
            .map_err(|_| Error::ParseError("waveform-index leaf cache is unavailable".into()))?
            .insert(key, leaf.clone());
        Ok(leaf)
    }

    pub(crate) fn load_root_summary(&self, channel: usize, block: usize) -> Result<RootDirEntry> {
        self.directory
            .get(channel)
            .and_then(|blocks| blocks.get(block))
            .copied()
            .ok_or_else(|| Error::ParseError("block index out of bounds".to_string()))
    }

    fn validate_header(
        index_header: &IndexHeader,
        header: &CaptureMetadata,
        source_revision: u64,
    ) -> Result<()> {
        if index_header.source_revision != source_revision
            || index_header.total_samples != header.total_samples
            || index_header.total_blocks != header.total_blocks
            || index_header.samples_per_block != header.samples_per_block
            || index_header.samplerate_bits != header.samplerate_hz.to_bits()
            || index_header.total_channels
                != u64::try_from(header.total_probes)
                    .map_err(|_| Error::ParseError("capture channel count exceeds u64".into()))?
            || index_header.blocks_per_channel != header.total_blocks
        {
            return Err(Error::ParseError("waveform index is stale".to_string()));
        }
        Ok(())
    }

    fn read_header(file: &mut (impl Read + Seek)) -> Result<IndexHeader> {
        file.seek(SeekFrom::Start(0))?;
        let mut magic = [0_u8; 8];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(Error::ParseError(
                "invalid waveform-index root magic".to_string(),
            ));
        }
        let version = read_u32(file)?;
        if version != 7 {
            return Err(Error::ParseError(
                "unsupported waveform-index root version".to_string(),
            ));
        }
        let header_size = read_u32(file)?;
        if u64::from(header_size) != HEADER_SIZE {
            return Err(Error::ParseError(
                "unsupported waveform-index root header size".to_string(),
            ));
        }
        Ok(IndexHeader {
            source_revision: read_u64(file)?,
            total_samples: read_u64(file)?,
            total_blocks: read_u64(file)?,
            samples_per_block: read_u64(file)?,
            samplerate_bits: read_u64(file)?,
            total_channels: read_u64(file)?,
            blocks_per_channel: read_u64(file)?,
            dir_offset: read_u64(file)?,
            payload_offset: read_u64(file)?,
        })
    }

    fn read_directory(
        file: &mut (impl Read + Seek),
        header: &IndexHeader,
        channels: usize,
        blocks_per_channel: usize,
    ) -> Result<Vec<Vec<RootDirEntry>>> {
        file.seek(SeekFrom::Start(header.dir_offset))?;
        let mut directory = vec![vec![RootDirEntry::default(); blocks_per_channel]; channels];
        for channel_dir in &mut directory {
            for entry in channel_dir {
                *entry = Self::read_dir_entry(file)?;
            }
        }
        Ok(directory)
    }

    fn read_dir_entry(file: &mut impl Read) -> Result<RootDirEntry> {
        let offset = read_u64(file)?;
        let len = read_u64(file)?;
        let mut flags_buf = [0_u8; 8];
        file.read_exact(&mut flags_buf)?;
        let flags = flags_buf[0];
        let l3_toggle = read_u64(file)?;
        let l3_last = read_u64(file)?;
        Ok(RootDirEntry {
            offset,
            len,
            toggle: flags & 0b001 != 0,
            first: flags & 0b010 != 0,
            last: flags & 0b100 != 0,
            l3_toggle,
            l3_last,
        })
    }
}

// ---------------------------------------------------------------------------
// Chunk codec — shared by IndexWriter (serialize) and IndexReader (deserialize)
// ---------------------------------------------------------------------------

fn serialize_leaf(leaf: &BlockIndex) -> Vec<u8> {
    let active = leaf.levels.is_some();
    let mut out = Vec::new();
    push_u32(&mut out, leaf.valid_samples);
    out.push((leaf.first as u8) | ((leaf.last as u8) << 1) | ((active as u8) << 2));
    out.extend_from_slice(&[0, 0, 0]);
    if let Some(levels) = &leaf.levels {
        push_u64_slice(&mut out, &levels.l1_toggle);
        push_u64_slice(&mut out, &levels.l1_last);
        push_u64_slice(&mut out, &levels.l2_toggle);
        push_u64_slice(&mut out, &levels.l2_last);
        push_u64(&mut out, levels.l3_toggle);
        push_u64(&mut out, levels.l3_last);
    }
    out
}

/// Decodes one serialized leaf artifact into fixed-size summary arrays.
fn leaf_view(data: &[u8]) -> Result<LeafView> {
    let truncated = || Error::ParseError("truncated waveform-index leaf".to_string());
    let (chunk_header, payload) = data.split_at_checked(8).ok_or_else(truncated)?;
    let valid_samples = u32::from_le_bytes(
        chunk_header[..4]
            .try_into()
            .expect("chunk header is 8 bytes"),
    );
    let flags = chunk_header[4];

    let levels = if flags & 0b100 != 0 {
        const LEVEL_WORDS: usize = 2 * L1_WORDS + 2 * L2_WORDS + 2;
        let payload = payload.get(..LEVEL_WORDS * 8).ok_or_else(truncated)?;
        let words = payload
            .as_chunks::<8>()
            .0
            .iter()
            .map(|word| u64::from_le_bytes(*word))
            .collect::<Vec<_>>();
        let (l1_toggle, rest) = words.split_at(L1_WORDS);
        let (l1_last, rest) = rest.split_at(L1_WORDS);
        let (l2_toggle, rest) = rest.split_at(L2_WORDS);
        let (l2_last, rest) = rest.split_at(L2_WORDS);
        Some(LevelsView {
            l1_toggle: Arc::from(l1_toggle),
            l1_last: Arc::from(l1_last),
            l2_toggle: Arc::from(l2_toggle),
            l2_last: Arc::from(l2_last),
            l3_toggle: rest[0],
        })
    } else {
        None
    };

    Ok(LeafView {
        valid_samples,
        first: flags & 0b001 != 0,
        last: flags & 0b010 != 0,
        levels,
    })
}

// ---------------------------------------------------------------------------
// Low-level I/O helpers
// ---------------------------------------------------------------------------

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64_slice(out: &mut Vec<u8>, values: &[u64]) {
    for value in values {
        push_u64(out, *value);
    }
}

fn read_u32(file: &mut impl Read) -> Result<u32> {
    let mut buf = [0_u8; 4];
    file.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64(file: &mut impl Read) -> Result<u64> {
    let mut buf = [0_u8; 8];
    file.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn write_u32(file: &mut impl Write, value: u32) -> Result<()> {
    file.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn write_u64(file: &mut impl Write, value: u64) -> Result<()> {
    file.write_all(&value.to_le_bytes())?;
    Ok(())
}

fn root_key(identity: SourceIdentity) -> Result<ArtifactKey> {
    artifact_key("waveform-index-root-v1", identity, None, None)
}

fn leaf_key(identity: SourceIdentity, channel: usize, block: usize) -> Result<ArtifactKey> {
    artifact_key(
        "waveform-index-leaf-v1",
        identity,
        Some(channel as u64),
        Some(block as u64),
    )
}

fn artifact_key(
    namespace: &str,
    identity: SourceIdentity,
    channel: Option<u64>,
    block: Option<u64>,
) -> Result<ArtifactKey> {
    let namespace = ArtifactNamespace::new(namespace).map_err(repository_error)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(namespace.as_str().as_bytes());
    hasher.update(identity.as_bytes());
    if let Some(channel) = channel {
        hasher.update(&channel.to_le_bytes());
    }
    if let Some(block) = block {
        hasher.update(&block.to_le_bytes());
    }
    Ok(ArtifactKey::new(
        namespace,
        SourceIdentity::from_bytes(*hasher.finalize().as_bytes()),
    ))
}

fn publish(repository: &dyn ArtifactRepository, key: ArtifactKey, bytes: &[u8]) -> Result<()> {
    let mut writer = repository.begin_write(key).map_err(repository_error)?;
    writer.write_at(0, bytes).map_err(repository_error)?;
    writer
        .truncate(bytes.len() as u64)
        .map_err(repository_error)?;
    writer.flush().map_err(repository_error)?;
    writer.publish().map_err(repository_error)
}

fn read(repository: &dyn ArtifactRepository, key: &ArtifactKey) -> Result<Option<Vec<u8>>> {
    let Some(mut reader) = repository.open(key).map_err(repository_error)? else {
        return Ok(None);
    };
    let length = reader.len().map_err(repository_error)?;
    let length = usize::try_from(length)
        .map_err(|_| Error::ParseError("waveform index artifact is too large".into()))?;
    let mut bytes = vec![0; length];
    let mut copied = 0;
    while copied < bytes.len() {
        let count = reader
            .read_at(copied as u64, &mut bytes[copied..])
            .map_err(repository_error)?;
        if count == 0 {
            return Err(Error::ParseError(
                "waveform index artifact is truncated".into(),
            ));
        }
        copied += count;
    }
    Ok(Some(bytes))
}

fn repository_error(error: RepositoryError) -> Error {
    Error::ParseError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::types::{BlockIndex, BlockLevels, bit, set_bit};
    use super::*;

    /// Copies serialized bytes into an 8-byte-aligned buffer, as `leaf_view`
    /// requires.
    fn aligned(data: &[u8]) -> Vec<u64> {
        let mut buf = vec![0_u64; data.len().div_ceil(8)];
        for (word, chunk) in buf.iter_mut().zip(data.chunks(8)) {
            let mut bytes = [0_u8; 8];
            bytes[..chunk.len()].copy_from_slice(chunk);
            *word = u64::from_le_bytes(bytes);
        }
        buf
    }

    fn as_bytes(buf: &[u64], len: usize) -> &[u8] {
        // SAFETY: reinterpreting u64s as bytes is always valid; len is
        // bounded by the buffer size.
        unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len.min(buf.len() * 8)) }
    }

    #[test]
    fn chunk_round_trips_active_leaf() {
        let mut lvl = BlockLevels::zeroed();
        set_bit(&mut lvl.l1_toggle[0], 0);
        set_bit(&mut lvl.l2_toggle[0], 0);
        set_bit(&mut lvl.l3_toggle, 0);
        let leaf = BlockIndex {
            valid_samples: 16,
            first: false,
            last: true,
            levels: Some(lvl),
        };
        let data = serialize_leaf(&leaf);
        let buf = aligned(&data);
        let decoded = leaf_view(as_bytes(&buf, data.len())).expect("leaf should decode");
        assert_eq!(decoded.valid_samples, 16);
        assert!(!decoded.first);
        assert!(decoded.last);
        let lvl = decoded
            .levels
            .as_ref()
            .expect("decoded leaf should be active");
        assert!(bit(lvl.l1_toggle[0], 0));
        assert!(bit(lvl.l2_toggle[0], 0));
        assert!(bit(lvl.l3_toggle, 0));
    }

    #[test]
    fn chunk_round_trips_constant_leaf() {
        let leaf = BlockIndex {
            valid_samples: 64,
            first: true,
            last: true,
            levels: None,
        };
        let data = serialize_leaf(&leaf);
        let buf = aligned(&data);
        let decoded = leaf_view(as_bytes(&buf, data.len())).expect("leaf should decode");
        assert!(decoded.levels.is_none());
        assert!(decoded.first);
        assert!(decoded.last);
    }

    #[test]
    fn root_header_preserves_counts_above_the_wasm32_address_range() {
        let count = u64::from(u32::MAX) + 41;
        let header = IndexHeader {
            source_revision: 1,
            total_samples: count + 2,
            total_blocks: count,
            samples_per_block: 16,
            samplerate_bits: 1_000_000_f64.to_bits(),
            total_channels: count + 1,
            blocks_per_channel: count,
            dir_offset: HEADER_SIZE,
            payload_offset: HEADER_SIZE + 8,
        };
        let mut bytes = Vec::new();

        IndexWriter::write_header(&mut bytes, &header).unwrap();
        let decoded = IndexReader::read_header(&mut Cursor::new(bytes)).unwrap();

        assert_eq!(decoded.total_channels, count + 1);
        assert_eq!(decoded.blocks_per_channel, count);
        assert_eq!(decoded.total_blocks, count);
    }

    #[test]
    fn reader_reuses_decoded_leaf_views() -> Result<()> {
        let repository: Arc<dyn ArtifactRepository> =
            Arc::new(crate::MemoryArtifactRepository::new());
        let identity = SourceIdentity::from_bytes([7; 32]);
        let metadata = CaptureMetadata {
            total_probes: 1,
            samplerate: "1 MHz".into(),
            samplerate_hz: 1_000_000.0,
            sample_period: 0.000_001,
            total_samples: 16,
            total_blocks: 1,
            samples_per_block: 16,
            probe_names: vec!["0".into()],
            trigger_sample: None,
        };
        let mut levels = BlockLevels::zeroed();
        set_bit(&mut levels.l1_toggle[0], 0);
        let leaf = BlockIndex {
            valid_samples: 16,
            first: false,
            last: true,
            levels: Some(levels),
        };
        let mut writer = IndexWriter::create(Arc::clone(&repository), identity, &metadata, 1)?;
        writer.write_block(0, 0, &leaf)?;
        writer.finish()?;

        let reader = IndexReader::open(repository, identity, metadata, 1)?;
        let first = reader.load_leaf(0, 0)?;
        let second = reader.load_leaf(0, 0)?;
        let first_levels = first.levels.expect("active leaf should have levels");
        let second_levels = second.levels.expect("active leaf should have levels");
        assert!(Arc::ptr_eq(
            &first_levels.l1_toggle,
            &second_levels.l1_toggle,
        ));
        Ok(())
    }
}
