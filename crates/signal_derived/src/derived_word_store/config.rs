use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use platform_artifacts::{
    ArtifactRepository, MemoryArtifactRepository, SystemUnixTimeSource, UnixTimeSource,
};
use platform_runtime::{InlineWorkExecutor, WorkExecutor};

pub(crate) const DEFAULT_MAX_WORDS_PER_BLOCK: usize = 131_072;

/// Platform-neutral sizing knobs for repository-backed encoded blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockCodecConfig {
    /// Maximum words encoded into one immutable block.
    pub max_words: usize,
    /// Word interval between in-block seek restart points.
    pub restart_interval: usize,
    /// Maximum encoded payload bytes per block.
    pub max_payload_bytes: usize,
    /// Maximum gap that permits two words to share a block.
    pub max_inter_word_gap_ns: u64,
    /// Maximum timestamp span that permits two words to share a block.
    pub max_timestamp_span_ns: u64,
}

impl Default for BlockCodecConfig {
    fn default() -> Self {
        Self {
            max_words: DEFAULT_MAX_WORDS_PER_BLOCK,
            restart_interval: 512,
            max_payload_bytes: 1024 * 1024,
            max_inter_word_gap_ns: 1_000_000,
            max_timestamp_span_ns: u64::MAX,
        }
    }
}

#[derive(Clone)]
pub struct PersistentStoreConfig {
    /// Stable 256-bit identity for this persistent derived-data entry.
    pub cache_key: [u8; 32],
    /// Cache cleanup capacity budget in bytes.
    pub max_cache_bytes: u64,
    /// Repository that owns encoded blocks, indexes, and manifests.
    pub artifact_repository: Arc<dyn ArtifactRepository>,
    /// Wall clock used for durable LRU metadata.
    pub time_source: Arc<dyn UnixTimeSource>,
}

impl PersistentStoreConfig {
    /// Creates persistent configuration with in-memory storage defaults.
    ///
    /// # Parameters
    /// - `cache_key`: Stable identity that namespaces all persistent artifacts.
    pub fn new(cache_key: [u8; 32]) -> Self {
        Self {
            cache_key,
            max_cache_bytes: DEFAULT_MAX_PERSISTENT_CACHE_BYTES,
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
            time_source: Arc::new(SystemUnixTimeSource),
        }
    }

    /// Replaces the repository that owns persistent artifacts.
    ///
    /// # Parameters
    ///
    /// - `repository`: Artifact repository used for this cache entry.
    pub fn with_artifact_repository(mut self, repository: Arc<dyn ArtifactRepository>) -> Self {
        self.artifact_repository = repository;
        self
    }

    /// Replaces the wall-clock source used for durable LRU metadata.
    ///
    /// # Parameters
    ///
    /// - `time_source`: Injectable Unix-epoch timestamp source.
    pub fn with_time_source(mut self, time_source: Arc<dyn UnixTimeSource>) -> Self {
        self.time_source = time_source;
        self
    }
}

impl fmt::Debug for PersistentStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PersistentStoreConfig")
            .field("cache_key", &self.cache_key)
            .field("max_cache_bytes", &self.max_cache_bytes)
            .field(
                "repository_capabilities",
                &self.artifact_repository.capabilities(),
            )
            .field("time_source", &"UnixTimeSource")
            .finish()
    }
}

#[derive(Clone)]
pub struct LiveStoreConfig {
    /// Prefix used to derive identities for non-persistent stores.
    pub cache_key_prefix: [u8; 16],
    /// Encoding and block-boundary constraints.
    pub block: BlockCodecConfig,
    /// Appends before the live hot tail is republished.
    pub hot_tail_publish_words: usize,
    /// Maximum time between live hot-tail publications.
    pub hot_tail_publish_interval: Duration,
    /// Optional persistent-cache configuration after finalization.
    pub persistence: Option<PersistentStoreConfig>,
    /// Host capability used to encode immutable blocks.
    pub work_executor: Arc<dyn WorkExecutor>,
    /// Repository used by non-persistent live stores.
    pub artifact_repository: Arc<dyn ArtifactRepository>,
}

impl fmt::Debug for LiveStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveStoreConfig")
            .field("cache_key_prefix", &self.cache_key_prefix)
            .field("block", &self.block)
            .field("hot_tail_publish_words", &self.hot_tail_publish_words)
            .field("hot_tail_publish_interval", &self.hot_tail_publish_interval)
            .field("persistence", &self.persistence)
            .field(
                "work_executor_parallelism",
                &self.work_executor.available_parallelism(),
            )
            .field(
                "repository_capabilities",
                &self.artifact_repository.capabilities(),
            )
            .finish()
    }
}

impl LiveStoreConfig {
    /// Selects the bounded executor used to encode finalized word blocks.
    ///
    /// # Parameters
    ///
    /// - `executor`: Host capability for asynchronous block encoding.
    pub fn with_work_executor(mut self, executor: Arc<dyn WorkExecutor>) -> Self {
        self.work_executor = executor;
        self
    }

    /// Selects the repository that owns finalized blocks, indexes, and manifests.
    ///
    /// # Parameters
    ///
    /// - `repository`: Artifact repository for live and persistent artifacts.
    pub fn with_artifact_repository(mut self, repository: Arc<dyn ArtifactRepository>) -> Self {
        if let Some(persistence) = &mut self.persistence {
            persistence.artifact_repository = Arc::clone(&repository);
        }
        self.artifact_repository = repository;
        self
    }
}

impl Default for LiveStoreConfig {
    fn default() -> Self {
        Self {
            cache_key_prefix: [0; 16],
            block: BlockCodecConfig::default(),
            hot_tail_publish_words: DEFAULT_HOT_TAIL_PUBLISH_WORDS,
            hot_tail_publish_interval: DEFAULT_HOT_TAIL_PUBLISH_INTERVAL,
            persistence: None,
            work_executor: Arc::new(InlineWorkExecutor),
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
        }
    }
}

const DEFAULT_HOT_TAIL_PUBLISH_WORDS: usize = 262_144;
const DEFAULT_HOT_TAIL_PUBLISH_INTERVAL: Duration = Duration::from_millis(50);
const DEFAULT_MAX_PERSISTENT_CACHE_BYTES: u64 = 50 * 1024 * 1024 * 1024;
