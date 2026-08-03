use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use super::config::PersistentStoreConfig;
use super::format::{BlockDirectoryEntry, FORMAT_VERSION};
use super::presence::{WordPresenceIndex, WordSummaryRecord};
use super::store::{StoreError, StoreResult};
use crate::{
    ArtifactKey, ArtifactNamespace, ArtifactRepository, ByteRange, RepositoryError, SourceIdentity,
    read_artifact_region,
};

const INDEX_MAGIC: &[u8; 8] = b"DWRIDX1\0";
const MANIFEST_MAGIC: &[u8; 8] = b"DWRMAN1\0";
const INDEX_VERSION: u32 = 4;
const MANIFEST_VERSION: u32 = 1;
const INDEX_HEADER_SIZE: usize = 104;
const INDEX_RECORD_SIZE: usize = 64;
const SUMMARY_RECORD_SIZE: usize = 40;
const MANIFEST_SIZE: usize = 96;

const INDEX_NAMESPACE: &str = "derived-word-index-v1";
const MANIFEST_NAMESPACE: &str = "derived-word-manifest-v1";
const BLOCK_NAMESPACE_PREFIX: &str = "derived-word-blocks-v1-";

#[derive(Debug)]
pub(crate) struct PersistentIndex {
    pub(crate) directory: Vec<BlockDirectoryEntry>,
    pub(crate) presence: WordPresenceIndex,
    pub(crate) committed_word_count: u64,
    pub(crate) committed_data_len: u64,
    pub(crate) first_timestamp_ns: Option<u64>,
    pub(crate) last_timestamp_ns: Option<u64>,
}

pub(crate) struct Publication<'a> {
    pub directory: &'a [BlockDirectoryEntry],
    pub presence: &'a WordPresenceIndex,
    pub committed_word_count: u64,
    pub committed_data_len: u64,
    pub first_timestamp_ns: Option<u64>,
    pub last_timestamp_ns: Option<u64>,
    pub created_unix_ns: u64,
}

#[derive(Debug, Clone, Copy)]
struct Manifest {
    cache_key: [u8; 32],
    data_len: u64,
    index_len: u64,
    word_count: u64,
    created_unix_ns: u64,
    accessed_unix_ns: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PersistentCacheStats {
    pub entries: usize,
    pub total_bytes: u64,
    pub removed_entries: usize,
    pub removed_bytes: u64,
}

/// Read-only diagnostics for one valid persistent derived-data entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistentCacheEntrySnapshot {
    pub total_bytes: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub word_count: u64,
    pub block_count: usize,
    pub first_timestamp_ns: Option<u64>,
    pub last_timestamp_ns: Option<u64>,
}

/// Incremental removal of every derived-word artifact in a repository.
///
/// Cooperative hosts advance this operation with a bounded artifact budget so
/// cache cleanup never monopolizes their UI thread. Threaded hosts may instead
/// use [`clear_cache`] on a host worker.
pub struct PersistentCacheClearTask {
    repository: Arc<dyn ArtifactRepository>,
    pending: VecDeque<crate::ArtifactMetadata>,
    removed_entries: usize,
    removed_bytes: u64,
}

impl PersistentCacheClearTask {
    pub fn new(repository: Arc<dyn ArtifactRepository>) -> StoreResult<Self> {
        let mut artifacts = BTreeMap::new();
        let mut cache_keys = BTreeSet::new();
        for namespace in [manifest_namespace()?, index_namespace()?] {
            for metadata in repository.entries(&namespace)? {
                cache_keys.insert(*metadata.key.identity().as_bytes());
                artifacts.insert(metadata.key.clone(), metadata);
            }
        }
        for namespace in repository.namespaces()? {
            let Some(encoded) = namespace.as_str().strip_prefix(BLOCK_NAMESPACE_PREFIX) else {
                continue;
            };
            let Some(cache_key) = parse_hex_key(encoded) else {
                continue;
            };
            cache_keys.insert(cache_key);
            for metadata in repository.entries(&namespace)? {
                artifacts.insert(metadata.key.clone(), metadata);
            }
        }
        Ok(Self {
            repository,
            pending: artifacts.into_values().collect(),
            removed_entries: cache_keys.len(),
            removed_bytes: 0,
        })
    }

    /// Removes at most `artifact_budget` artifacts and returns final statistics
    /// once no work remains.
    pub fn advance(&mut self, artifact_budget: usize) -> StoreResult<Option<PersistentCacheStats>> {
        for _ in 0..artifact_budget.max(1) {
            let Some(metadata) = self.pending.pop_front() else {
                return Ok(Some(self.stats()));
            };
            self.repository.remove(&metadata.key)?;
            self.removed_bytes = self.removed_bytes.saturating_add(metadata.length);
        }
        if self.pending.is_empty() {
            Ok(Some(self.stats()))
        } else {
            Ok(None)
        }
    }

    fn stats(&self) -> PersistentCacheStats {
        PersistentCacheStats {
            removed_entries: self.removed_entries,
            removed_bytes: self.removed_bytes,
            ..PersistentCacheStats::default()
        }
    }
}

pub fn cleanup_cache(
    repository: &Arc<dyn ArtifactRepository>,
    max_total_bytes: u64,
    pinned_keys: &[[u8; 32]],
) -> StoreResult<PersistentCacheStats> {
    let mut entries = Vec::new();
    let mut stats = PersistentCacheStats::default();
    let manifests = repository.entries(&manifest_namespace()?)?;
    let published_keys = manifests
        .iter()
        .map(|metadata| *metadata.key.identity().as_bytes())
        .collect::<BTreeSet<_>>();
    for metadata in manifests {
        let cache_key = *metadata.key.identity().as_bytes();
        let config =
            PersistentStoreConfig::new(cache_key).with_artifact_repository(Arc::clone(repository));
        match inspect_cache_entry(&config) {
            Ok(Some(snapshot)) => {
                let manifest = read_manifest(&config)?
                    .ok_or_else(|| StoreError::Persistent("listed manifest disappeared".into()))?;
                stats.entries += 1;
                stats.total_bytes = stats.total_bytes.saturating_add(snapshot.total_bytes);
                entries.push((manifest.accessed_unix_ns, cache_key, snapshot.total_bytes));
            }
            Ok(None) => {}
            Err(_) => {
                let removed = remove_cache_artifacts(&config)?;
                stats.removed_entries += 1;
                stats.removed_bytes = stats.removed_bytes.saturating_add(removed);
            }
        }
    }
    for cache_key in all_cache_keys(repository)? {
        if published_keys.contains(&cache_key) {
            continue;
        }
        let config =
            PersistentStoreConfig::new(cache_key).with_artifact_repository(Arc::clone(repository));
        let removed = remove_cache_artifacts(&config)?;
        if removed > 0 {
            stats.removed_entries += 1;
            stats.removed_bytes = stats.removed_bytes.saturating_add(removed);
        }
    }
    if stats.total_bytes > max_total_bytes {
        entries.sort_by_key(|entry| entry.0);
        for (_, cache_key, bytes) in entries {
            if stats.total_bytes <= max_total_bytes {
                break;
            }
            if pinned_keys.contains(&cache_key) {
                continue;
            }
            let config = PersistentStoreConfig::new(cache_key)
                .with_artifact_repository(Arc::clone(repository));
            remove_cache_artifacts(&config)?;
            stats.entries -= 1;
            stats.total_bytes = stats.total_bytes.saturating_sub(bytes);
            stats.removed_entries += 1;
            stats.removed_bytes = stats.removed_bytes.saturating_add(bytes);
        }
    }
    Ok(stats)
}

pub fn clear_cache(repository: &Arc<dyn ArtifactRepository>) -> StoreResult<PersistentCacheStats> {
    let mut stats = PersistentCacheStats::default();
    for cache_key in all_cache_keys(repository)? {
        let config =
            PersistentStoreConfig::new(cache_key).with_artifact_repository(Arc::clone(repository));
        stats.removed_bytes = stats
            .removed_bytes
            .saturating_add(remove_cache_artifacts(&config)?);
        stats.removed_entries += 1;
    }
    Ok(stats)
}

pub fn clear_cache_entry(config: &PersistentStoreConfig) -> StoreResult<PersistentCacheStats> {
    if config
        .artifact_repository
        .open(&manifest_key(config)?)?
        .is_none()
    {
        return Ok(PersistentCacheStats::default());
    }
    let bytes = remove_cache_artifacts(config)?;
    Ok(PersistentCacheStats {
        removed_entries: 1,
        removed_bytes: bytes,
        ..PersistentCacheStats::default()
    })
}

/// Inspects an entry without updating its LRU timestamp or deleting invalid data.
pub fn inspect_cache_entry(
    config: &PersistentStoreConfig,
) -> StoreResult<Option<PersistentCacheEntrySnapshot>> {
    let Some(manifest) = read_manifest(config)? else {
        return Ok(None);
    };
    let index_bytes = read_required(config.artifact_repository.as_ref(), &index_key(config)?)?;
    if index_bytes.len() as u64 != manifest.index_len {
        return Err(StoreError::Persistent(
            "persistent index length mismatch".into(),
        ));
    }
    let index = decode_index(&index_bytes, config.cache_key)?;
    validate_persistent_generation(config, manifest, &index)?;
    let data_bytes = index.committed_data_len;
    let index_bytes = index_bytes.len() as u64;
    Ok(Some(PersistentCacheEntrySnapshot {
        total_bytes: data_bytes
            .saturating_add(index_bytes)
            .saturating_add(MANIFEST_SIZE as u64),
        data_bytes,
        index_bytes,
        word_count: index.committed_word_count,
        block_count: index.directory.len(),
        first_timestamp_ns: index.first_timestamp_ns,
        last_timestamp_ns: index.last_timestamp_ns,
    }))
}

pub(crate) fn publish(
    config: &PersistentStoreConfig,
    publication: Publication<'_>,
) -> StoreResult<()> {
    let index_bytes = encode_index(
        config.cache_key,
        publication.directory,
        publication.presence.leaves(),
        publication.committed_word_count,
        publication.committed_data_len,
        publication.first_timestamp_ns,
        publication.last_timestamp_ns,
    )?;
    publish_bytes(
        config.artifact_repository.as_ref(),
        index_key(config)?,
        &index_bytes,
    )?;

    let now = config.time_source.now_unix_ns();
    let manifest = Manifest {
        cache_key: config.cache_key,
        data_len: publication.committed_data_len,
        index_len: index_bytes.len() as u64,
        word_count: publication.committed_word_count,
        created_unix_ns: publication.created_unix_ns,
        accessed_unix_ns: now,
    };
    publish_bytes(
        config.artifact_repository.as_ref(),
        manifest_key(config)?,
        &encode_manifest(manifest),
    )
}

pub(crate) fn open(config: &PersistentStoreConfig) -> StoreResult<Option<PersistentIndex>> {
    match open_validated(config) {
        Ok(result) => Ok(result),
        Err(error) => {
            let _ = remove_cache_artifacts(config);
            Err(error)
        }
    }
}

fn open_validated(config: &PersistentStoreConfig) -> StoreResult<Option<PersistentIndex>> {
    let Some(mut manifest) = read_manifest(config)? else {
        return Ok(None);
    };
    let index_bytes = read_required(config.artifact_repository.as_ref(), &index_key(config)?)?;
    let index = decode_index(&index_bytes, config.cache_key)?;
    validate_persistent_generation(config, manifest, &index)?;
    manifest.accessed_unix_ns = config.time_source.now_unix_ns();
    publish_bytes(
        config.artifact_repository.as_ref(),
        manifest_key(config)?,
        &encode_manifest(manifest),
    )?;
    Ok(Some(index))
}

pub(crate) fn invalidate(config: &PersistentStoreConfig) -> StoreResult<()> {
    remove_cache_artifacts(config).map(|_| ())
}

pub(crate) fn block_key(store_identity: [u8; 32], sequence: u64) -> StoreResult<ArtifactKey> {
    let mut identity = [0_u8; 32];
    identity[..8].copy_from_slice(&sequence.to_le_bytes());
    Ok(ArtifactKey::new(
        block_namespace(store_identity)?,
        SourceIdentity::from_bytes(identity),
    ))
}

fn validate_persistent_generation(
    config: &PersistentStoreConfig,
    manifest: Manifest,
    index: &PersistentIndex,
) -> StoreResult<()> {
    if manifest.cache_key != config.cache_key
        || index.committed_word_count != manifest.word_count
        || index.committed_data_len != manifest.data_len
    {
        return Err(StoreError::Persistent(
            "persistent manifest metadata mismatch".into(),
        ));
    }
    validate_directory(&index.directory, manifest.data_len)?;
    Ok(())
}

fn read_manifest(config: &PersistentStoreConfig) -> StoreResult<Option<Manifest>> {
    let Some(bytes) = read_optional(config.artifact_repository.as_ref(), &manifest_key(config)?)?
    else {
        return Ok(None);
    };
    decode_manifest(&bytes).map(Some)
}

fn read_optional(
    repository: &dyn ArtifactRepository,
    key: &ArtifactKey,
) -> StoreResult<Option<Vec<u8>>> {
    let Some(mut reader) = repository.open(key)? else {
        return Ok(None);
    };
    let length = reader.len()?;
    let range = ByteRange::new(0, length).map_err(RepositoryError::from)?;
    let region = read_artifact_region(&mut *reader, range)?;
    Ok(Some(region.bytes().to_vec()))
}

fn read_required(repository: &dyn ArtifactRepository, key: &ArtifactKey) -> StoreResult<Vec<u8>> {
    read_optional(repository, key)?.ok_or_else(|| {
        StoreError::Persistent(format!(
            "required artifact in '{}' is missing",
            key.namespace().as_str()
        ))
    })
}

fn publish_bytes(
    repository: &dyn ArtifactRepository,
    key: ArtifactKey,
    bytes: &[u8],
) -> StoreResult<()> {
    let mut writer = repository.begin_write(key)?;
    writer.write_at(0, bytes)?;
    writer.truncate(bytes.len() as u64)?;
    writer.flush()?;
    writer.publish()?;
    Ok(())
}

fn remove_cache_artifacts(config: &PersistentStoreConfig) -> StoreResult<u64> {
    let repository = config.artifact_repository.as_ref();
    let mut removed_bytes = 0_u64;
    for namespace in [manifest_namespace()?, index_namespace()?] {
        let key = ArtifactKey::new(namespace, SourceIdentity::from_bytes(config.cache_key));
        if let Some(reader) = repository.open(&key)? {
            removed_bytes = removed_bytes.saturating_add(reader.len()?);
        }
        repository.remove(&key)?;
    }
    let block_namespace = block_namespace(config.cache_key)?;
    for metadata in repository.entries(&block_namespace)? {
        removed_bytes = removed_bytes.saturating_add(metadata.length);
        repository.remove(&metadata.key)?;
    }
    Ok(removed_bytes)
}

fn all_cache_keys(repository: &Arc<dyn ArtifactRepository>) -> StoreResult<BTreeSet<[u8; 32]>> {
    let mut keys = BTreeSet::new();
    for namespace in [manifest_namespace()?, index_namespace()?] {
        keys.extend(
            repository
                .entries(&namespace)?
                .into_iter()
                .map(|metadata| *metadata.key.identity().as_bytes()),
        );
    }
    for namespace in repository.namespaces()? {
        let Some(encoded) = namespace.as_str().strip_prefix(BLOCK_NAMESPACE_PREFIX) else {
            continue;
        };
        if let Some(key) = parse_hex_key(encoded) {
            keys.insert(key);
        }
    }
    Ok(keys)
}

fn manifest_key(config: &PersistentStoreConfig) -> StoreResult<ArtifactKey> {
    Ok(ArtifactKey::new(
        manifest_namespace()?,
        SourceIdentity::from_bytes(config.cache_key),
    ))
}

fn index_key(config: &PersistentStoreConfig) -> StoreResult<ArtifactKey> {
    Ok(ArtifactKey::new(
        index_namespace()?,
        SourceIdentity::from_bytes(config.cache_key),
    ))
}

fn manifest_namespace() -> StoreResult<ArtifactNamespace> {
    ArtifactNamespace::new(MANIFEST_NAMESPACE).map_err(StoreError::from)
}

fn index_namespace() -> StoreResult<ArtifactNamespace> {
    ArtifactNamespace::new(INDEX_NAMESPACE).map_err(StoreError::from)
}

fn block_namespace(identity: [u8; 32]) -> StoreResult<ArtifactNamespace> {
    ArtifactNamespace::new(format!("{BLOCK_NAMESPACE_PREFIX}{}", hex_key(&identity)))
        .map_err(StoreError::from)
}

fn encode_index(
    cache_key: [u8; 32],
    directory: &[BlockDirectoryEntry],
    summaries: &[WordSummaryRecord],
    word_count: u64,
    data_len: u64,
    first_timestamp_ns: Option<u64>,
    last_timestamp_ns: Option<u64>,
) -> StoreResult<Vec<u8>> {
    let directory_bytes = directory
        .len()
        .checked_mul(INDEX_RECORD_SIZE)
        .ok_or_else(|| StoreError::Persistent("index size overflow".into()))?;
    let summary_bytes = summaries
        .len()
        .checked_mul(SUMMARY_RECORD_SIZE)
        .ok_or_else(|| StoreError::Persistent("summary index size overflow".into()))?;
    let index_len = INDEX_HEADER_SIZE
        .checked_add(directory_bytes)
        .and_then(|length| length.checked_add(summary_bytes))
        .ok_or_else(|| StoreError::Persistent("index size overflow".into()))?;
    let summary_count = u64::try_from(summaries.len())
        .map_err(|_| StoreError::Persistent("summary count exceeds u64".into()))?;
    validate_summaries(directory, summaries, word_count)?;
    let mut bytes = vec![0u8; index_len];
    bytes[..8].copy_from_slice(INDEX_MAGIC);
    put_u32(&mut bytes, 8, INDEX_VERSION);
    put_u32(&mut bytes, 12, FORMAT_VERSION);
    bytes[16..48].copy_from_slice(&cache_key);
    put_u64(&mut bytes, 48, directory.len() as u64);
    put_u64(&mut bytes, 56, word_count);
    put_optional_u64(&mut bytes, 64, first_timestamp_ns);
    put_optional_u64(&mut bytes, 72, last_timestamp_ns);
    put_u64(&mut bytes, 80, data_len);
    put_u64(&mut bytes, 88, summary_count);
    for (index, entry) in directory.iter().enumerate() {
        let offset = INDEX_HEADER_SIZE + index * INDEX_RECORD_SIZE;
        put_u64(&mut bytes, offset, entry.sequence);
        put_u64(&mut bytes, offset + 8, entry.first_timestamp_ns);
        put_u64(&mut bytes, offset + 16, entry.last_timestamp_ns);
        put_u64(&mut bytes, offset + 24, entry.data_offset);
        put_u32(&mut bytes, offset + 32, entry.block_len);
        put_u32(&mut bytes, offset + 36, entry.word_count);
        bytes[offset + 40] = entry.value_bytes;
        bytes[offset + 41] = entry.flags;
    }
    let summaries_offset = INDEX_HEADER_SIZE + directory_bytes;
    for (index, summary) in summaries.iter().enumerate() {
        let offset = summaries_offset + index * SUMMARY_RECORD_SIZE;
        put_u64(&mut bytes, offset, summary.start_ns);
        put_u64(&mut bytes, offset + 8, summary.end_ns);
        put_u64(&mut bytes, offset + 16, summary.word_count);
        put_u64(&mut bytes, offset + 24, summary.first_block);
        put_u32(&mut bytes, offset + 32, summary.block_count);
    }
    let checksum = crate::crc32c::block_checksum(&bytes, 96);
    put_u32(&mut bytes, 96, checksum);
    Ok(bytes)
}

fn decode_index(bytes: &[u8], cache_key: [u8; 32]) -> StoreResult<PersistentIndex> {
    if bytes.len() < INDEX_HEADER_SIZE || &bytes[..8] != INDEX_MAGIC {
        return Err(StoreError::Persistent(
            "invalid persistent index header".into(),
        ));
    }
    if get_u32(bytes, 8)? != INDEX_VERSION || get_u32(bytes, 12)? != FORMAT_VERSION {
        return Err(StoreError::Persistent(
            "unsupported persistent index version".into(),
        ));
    }
    if bytes[16..48] != cache_key {
        return Err(StoreError::Persistent("index cache key mismatch".into()));
    }
    let expected_checksum = get_u32(bytes, 96)?;
    if crate::crc32c::block_checksum(bytes, 96) != expected_checksum {
        return Err(StoreError::Persistent(
            "persistent index checksum mismatch".into(),
        ));
    }
    let block_count = usize::try_from(get_u64(bytes, 48)?)
        .map_err(|_| StoreError::Persistent("block count exceeds usize".into()))?;
    let summary_count = usize::try_from(get_u64(bytes, 88)?)
        .map_err(|_| StoreError::Persistent("summary count exceeds usize".into()))?;
    let directory_bytes = block_count
        .checked_mul(INDEX_RECORD_SIZE)
        .ok_or_else(|| StoreError::Persistent("persistent index record size overflow".into()))?;
    let summary_bytes = summary_count
        .checked_mul(SUMMARY_RECORD_SIZE)
        .ok_or_else(|| StoreError::Persistent("persistent summary record size overflow".into()))?;
    let expected_len = INDEX_HEADER_SIZE
        .checked_add(directory_bytes)
        .and_then(|length| length.checked_add(summary_bytes))
        .ok_or_else(|| StoreError::Persistent("persistent index size overflow".into()))?;
    if bytes.len() != expected_len {
        return Err(StoreError::Persistent(
            "persistent index length mismatch".into(),
        ));
    }
    let mut directory = Vec::with_capacity(block_count);
    for index in 0..block_count {
        let offset = INDEX_HEADER_SIZE + index * INDEX_RECORD_SIZE;
        let entry = BlockDirectoryEntry {
            sequence: get_u64(bytes, offset)?,
            first_timestamp_ns: get_u64(bytes, offset + 8)?,
            last_timestamp_ns: get_u64(bytes, offset + 16)?,
            data_offset: get_u64(bytes, offset + 24)?,
            block_len: get_u32(bytes, offset + 32)?,
            word_count: get_u32(bytes, offset + 36)?,
            value_bytes: bytes[offset + 40],
            flags: bytes[offset + 41],
        };
        directory.push(entry);
    }
    let mut summaries = Vec::with_capacity(summary_count);
    let summaries_offset = INDEX_HEADER_SIZE + directory_bytes;
    for index in 0..summary_count {
        let offset = summaries_offset + index * SUMMARY_RECORD_SIZE;
        summaries.push(WordSummaryRecord {
            start_ns: get_u64(bytes, offset)?,
            end_ns: get_u64(bytes, offset + 8)?,
            word_count: get_u64(bytes, offset + 16)?,
            first_block: get_u64(bytes, offset + 24)?,
            block_count: get_u32(bytes, offset + 32)?,
        });
    }
    let committed_word_count = get_u64(bytes, 56)?;
    validate_summaries(&directory, &summaries, committed_word_count)?;
    let mut presence = WordPresenceIndex::new();
    for summary in summaries {
        presence.push(summary);
    }
    Ok(PersistentIndex {
        directory,
        presence,
        committed_word_count,
        committed_data_len: get_u64(bytes, 80)?,
        first_timestamp_ns: get_optional_u64(bytes, 64)?,
        last_timestamp_ns: get_optional_u64(bytes, 72)?,
    })
}

fn validate_summaries(
    directory: &[BlockDirectoryEntry],
    summaries: &[WordSummaryRecord],
    expected_word_count: u64,
) -> StoreResult<()> {
    let mut words_per_block = vec![0u64; directory.len()];
    let mut previous_start = None;
    for summary in summaries {
        let block = usize::try_from(summary.first_block)
            .map_err(|_| StoreError::Persistent("summary block exceeds usize".into()))?;
        if summary.word_count == 0
            || summary.start_ns > summary.end_ns
            || summary.block_count != 1
            || block >= directory.len()
            || previous_start.is_some_and(|start| summary.start_ns < start)
        {
            return Err(StoreError::Persistent(
                "invalid persistent presence summary".into(),
            ));
        }
        words_per_block[block] = words_per_block[block].saturating_add(summary.word_count);
        previous_start = Some(summary.start_ns);
    }
    if words_per_block
        .iter()
        .zip(directory)
        .any(|(&count, entry)| count != u64::from(entry.word_count))
        || words_per_block
            .iter()
            .copied()
            .fold(0u64, u64::saturating_add)
            != expected_word_count
    {
        return Err(StoreError::Persistent(
            "presence summary word count mismatch".into(),
        ));
    }
    Ok(())
}

fn validate_directory(directory: &[BlockDirectoryEntry], data_len: u64) -> StoreResult<()> {
    let mut expected_offset = 0_u64;
    for (index, entry) in directory.iter().enumerate() {
        if entry.sequence != index as u64
            || entry.data_offset != expected_offset
            || entry.word_count == 0
            || entry.first_timestamp_ns > entry.last_timestamp_ns
        {
            return Err(StoreError::Persistent("invalid block directory".into()));
        }
        expected_offset = expected_offset
            .checked_add(u64::from(entry.block_len))
            .ok_or_else(|| StoreError::Persistent("block directory offset overflow".into()))?;
    }
    if expected_offset != data_len {
        return Err(StoreError::Persistent(
            "block directory data length mismatch".into(),
        ));
    }
    Ok(())
}

fn encode_manifest(manifest: Manifest) -> [u8; MANIFEST_SIZE] {
    let mut bytes = [0u8; MANIFEST_SIZE];
    bytes[..8].copy_from_slice(MANIFEST_MAGIC);
    put_u32(&mut bytes, 8, MANIFEST_VERSION);
    put_u32(&mut bytes, 12, FORMAT_VERSION);
    bytes[16..48].copy_from_slice(&manifest.cache_key);
    put_u64(&mut bytes, 48, manifest.data_len);
    put_u64(&mut bytes, 56, manifest.index_len);
    put_u64(&mut bytes, 64, manifest.word_count);
    put_u64(&mut bytes, 72, manifest.created_unix_ns);
    put_u64(&mut bytes, 80, manifest.accessed_unix_ns);
    let checksum = crate::crc32c::block_checksum(&bytes, 88);
    put_u32(&mut bytes, 88, checksum);
    bytes
}

fn decode_manifest(bytes: &[u8]) -> StoreResult<Manifest> {
    if bytes.len() != MANIFEST_SIZE || &bytes[..8] != MANIFEST_MAGIC {
        return Err(StoreError::Persistent("invalid persistent manifest".into()));
    }
    if get_u32(bytes, 8)? != MANIFEST_VERSION || get_u32(bytes, 12)? != FORMAT_VERSION {
        return Err(StoreError::Persistent(
            "unsupported persistent manifest version".into(),
        ));
    }
    let expected_checksum = get_u32(bytes, 88)?;
    if crate::crc32c::block_checksum(bytes, 88) != expected_checksum {
        return Err(StoreError::Persistent(
            "persistent manifest checksum mismatch".into(),
        ));
    }
    let mut cache_key = [0u8; 32];
    cache_key.copy_from_slice(&bytes[16..48]);
    Ok(Manifest {
        cache_key,
        data_len: get_u64(bytes, 48)?,
        index_len: get_u64(bytes, 56)?,
        word_count: get_u64(bytes, 64)?,
        created_unix_ns: get_u64(bytes, 72)?,
        accessed_unix_ns: get_u64(bytes, 80)?,
    })
}

fn hex_key(key: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for &byte in key {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn parse_hex_key(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut key = [0_u8; 32];
    let (pairs, []) = value.as_bytes().as_chunks::<2>() else {
        return None;
    };
    for (output, pair) in key.iter_mut().zip(pairs) {
        *output = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
    }
    Some(key)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn put_optional_u64(bytes: &mut [u8], offset: usize, value: Option<u64>) {
    put_u64(bytes, offset, value.unwrap_or(u64::MAX));
}

fn get_u32(bytes: &[u8], offset: usize) -> StoreResult<u32> {
    let raw: [u8; 4] = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| StoreError::Persistent("truncated persistent metadata".into()))?
        .try_into()
        .unwrap();
    Ok(u32::from_le_bytes(raw))
}

fn get_u64(bytes: &[u8], offset: usize) -> StoreResult<u64> {
    let raw: [u8; 8] = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| StoreError::Persistent("truncated persistent metadata".into()))?
        .try_into()
        .unwrap();
    Ok(u64::from_le_bytes(raw))
}

fn get_optional_u64(bytes: &[u8], offset: usize) -> StoreResult<Option<u64>> {
    Ok(match get_u64(bytes, offset)? {
        u64::MAX => None,
        value => Some(value),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::derived_word_store::{
        AnnotationQuery, IndexedAnnotationStore, IndexedAnnotationWriter, LiveStoreConfig,
    };
    use crate::events::Word;
    use crate::{ArtifactRepository, MemoryArtifactRepository};

    #[test]
    fn index_header_preserves_counts_above_the_wasm32_address_range() {
        let count = u64::from(u32::MAX) + 37;
        let mut bytes = vec![0_u8; INDEX_HEADER_SIZE];

        put_u64(&mut bytes, 48, count);
        put_u64(&mut bytes, 88, count + 1);

        assert_eq!(get_u64(&bytes, 48).unwrap(), count);
        assert_eq!(get_u64(&bytes, 88).unwrap(), count + 1);
    }

    #[test]
    fn corrupt_manifest_invalidates_the_complete_repository_generation() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let cache_key = [0xC1; 32];
        let persistent =
            PersistentStoreConfig::new(cache_key).with_artifact_repository(Arc::clone(&repository));
        let config = LiveStoreConfig {
            persistence: Some(persistent.clone()),
            ..LiveStoreConfig::default()
        };
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer.append(Word::new(0x42, 100)).unwrap();
        writer.finish().unwrap();
        drop(store);

        publish_bytes(
            repository.as_ref(),
            manifest_key(&persistent).unwrap(),
            b"corrupt manifest",
        )
        .unwrap();

        assert!(IndexedAnnotationStore::open_persistent(&persistent).is_err());
        assert!(
            repository
                .open(&manifest_key(&persistent).unwrap())
                .unwrap()
                .is_none()
        );
        assert!(
            repository
                .open(&index_key(&persistent).unwrap())
                .unwrap()
                .is_none()
        );
        assert!(
            repository
                .entries(&block_namespace(cache_key).unwrap())
                .unwrap()
                .is_empty()
        );
        assert!(
            IndexedAnnotationStore::open_persistent(&persistent)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn opening_a_generation_defers_block_validation_until_the_block_is_queried() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let cache_key = [0xC3; 32];
        let persistent =
            PersistentStoreConfig::new(cache_key).with_artifact_repository(Arc::clone(&repository));
        let config = LiveStoreConfig {
            persistence: Some(persistent.clone()),
            ..LiveStoreConfig::default()
        };
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer.append(Word::new(0x42, 100)).unwrap();
        writer.finish().unwrap();
        drop(store);

        repository
            .remove(&block_key(cache_key, 0).unwrap())
            .unwrap();

        let reopened = IndexedAnnotationStore::open_persistent(&persistent)
            .expect("metadata should remain readable")
            .expect("the complete manifest and index should identify a cache hit");
        assert!(reopened.exact_window(0, 200, 8).is_err());
    }

    #[test]
    fn cleanup_reclaims_an_interrupted_generation_without_a_manifest() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let cache_key = [0xC2; 32];
        let key = block_key(cache_key, 0).unwrap();
        publish_bytes(repository.as_ref(), key.clone(), b"orphan").unwrap();

        let stats = cleanup_cache(&repository, u64::MAX, &[]).unwrap();

        assert_eq!(stats.removed_entries, 1);
        assert_eq!(stats.removed_bytes, 6);
        assert!(repository.open(&key).unwrap().is_none());
    }

    #[test]
    fn cooperative_clear_respects_its_artifact_budget() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let cache_key = [0xC4; 32];
        let namespace = block_namespace(cache_key).unwrap();
        for sequence in 0..3 {
            publish_bytes(
                repository.as_ref(),
                block_key(cache_key, sequence).unwrap(),
                &[sequence as u8],
            )
            .unwrap();
        }
        let mut task = PersistentCacheClearTask::new(Arc::clone(&repository)).unwrap();

        assert_eq!(task.advance(1).unwrap(), None);
        assert_eq!(repository.entries(&namespace).unwrap().len(), 2);
        assert_eq!(task.advance(1).unwrap(), None);
        let stats = task.advance(1).unwrap().expect("clear should finish");

        assert_eq!(stats.removed_entries, 1);
        assert_eq!(stats.removed_bytes, 3);
        assert!(repository.entries(&namespace).unwrap().is_empty());
    }
}
