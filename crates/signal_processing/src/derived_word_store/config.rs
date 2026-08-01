use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::{ArtifactRepository, InlineWorkExecutor, MemoryArtifactRepository, WorkExecutor};

/// Platform-neutral sizing knobs for repository-backed encoded blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockCodecConfig {
    pub max_words: usize,
    pub restart_interval: usize,
    pub max_payload_bytes: usize,
    pub max_inter_word_gap_ns: u64,
    pub max_timestamp_span_ns: u64,
}

impl Default for BlockCodecConfig {
    fn default() -> Self {
        Self {
            max_words: 32_768,
            restart_interval: 512,
            max_payload_bytes: 1024 * 1024,
            max_inter_word_gap_ns: 1_000_000,
            max_timestamp_span_ns: u64::MAX,
        }
    }
}

#[derive(Clone)]
pub struct PersistentStoreConfig {
    pub cache_key: [u8; 32],
    pub max_cache_bytes: u64,
    pub artifact_repository: Arc<dyn ArtifactRepository>,
}

impl PersistentStoreConfig {
    pub fn new(cache_key: [u8; 32]) -> Self {
        Self {
            cache_key,
            max_cache_bytes: DEFAULT_MAX_PERSISTENT_CACHE_BYTES,
            artifact_repository: Arc::new(MemoryArtifactRepository::new()),
        }
    }

    pub fn with_artifact_repository(mut self, repository: Arc<dyn ArtifactRepository>) -> Self {
        self.artifact_repository = repository;
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
            .finish()
    }
}

#[derive(Clone)]
pub struct LiveStoreConfig {
    pub cache_key_prefix: [u8; 16],
    pub block: BlockCodecConfig,
    pub hot_tail_publish_words: usize,
    pub hot_tail_publish_interval: Duration,
    pub persistence: Option<PersistentStoreConfig>,
    pub work_executor: Arc<dyn WorkExecutor>,
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
    pub fn with_work_executor(mut self, executor: Arc<dyn WorkExecutor>) -> Self {
        self.work_executor = executor;
        self
    }

    /// Selects the repository that owns finalized blocks, indexes, and manifests.
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
