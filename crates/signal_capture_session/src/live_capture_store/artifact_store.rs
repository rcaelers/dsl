use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use platform_artifacts::{
    ArtifactKey, ArtifactNamespace, ArtifactRepository, ByteRange, ByteRegion, RepositoryError,
    SourceIdentity, SystemUnixTimeSource, UnixTimeSource, read_artifact_region,
};
use signal_capture::{CaptureSampledChannel, CaptureSampledWindow, CaptureTransition, Error};

use super::implementation::{
    CaptureCursorItem, CaptureReclamationReport, CaptureRecoveryReport, CaptureSessionMetadata,
    CaptureSessionOutcome, CaptureStoreCursor, CaptureStoreDescriptor, CaptureStoreError,
    CaptureStoreManifest, CaptureStoreResult, CaptureStoreSnapshot, CaptureTimelineMetadata,
};
use crate::{
    CaptureChunk, CaptureChunkPayload, CaptureChunkWriter, CaptureSessionId, CaptureSessionPlan,
    CaptureWriteError,
};

const CAPTURE_MANIFEST_NAMESPACE: &str = "capture-manifest-v1";
const CAPTURE_CHUNK_NAMESPACE: &str = "capture-chunk-v1";
const CAPTURE_SESSION_NAMESPACE: &str = "capture-session-v1";
const CAPTURE_PLAN_NAMESPACE: &str = "capture-plan-v1";
const CHUNK_MAGIC: &[u8; 8] = b"DSLCHK01";
const CHUNK_FORMAT_VERSION: u16 = 1;

#[derive(Clone)]
pub struct CaptureStoreConfig {
    repository: Arc<dyn ArtifactRepository>,
    descriptor: CaptureStoreDescriptor,
    time_source: Arc<dyn UnixTimeSource>,
}

impl CaptureStoreConfig {
    /// Creates configuration for a new authoritative capture store.
    ///
    /// # Parameters
    /// - `repository`: Artifact repository that persists store artifacts.
    /// - `descriptor`: Session identity and ordered physical-channel table.
    pub fn new(
        repository: Arc<dyn ArtifactRepository>,
        descriptor: CaptureStoreDescriptor,
    ) -> Self {
        Self {
            repository,
            descriptor,
            time_source: Arc::new(SystemUnixTimeSource),
        }
    }

    /// Replaces the wall-clock source used for durable session metadata.
    ///
    /// # Parameters
    ///
    /// - `time_source`: Injectable wall-clock capability.
    pub fn with_time_source(mut self, time_source: Arc<dyn UnixTimeSource>) -> Self {
        self.time_source = time_source;
        self
    }

    /// Returns the configured artifact repository.
    pub fn repository(&self) -> Arc<dyn ArtifactRepository> {
        Arc::clone(&self.repository)
    }

    /// Returns the configured session descriptor.
    pub fn descriptor(&self) -> &CaptureStoreDescriptor {
        &self.descriptor
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedManifest {
    format_version: u16,
    session_id: String,
    channels: Vec<String>,
    generation: u64,
    first_sequence: u64,
    committed_chunks: u64,
    committed_samples: u64,
    committed_data_bytes: u64,
    finalized: bool,
}

impl PersistedManifest {
    fn descriptor(&self) -> CaptureStoreResult<CaptureStoreDescriptor> {
        if self.format_version != 1 {
            return Err(CaptureStoreError::Corrupt(format!(
                "unsupported capture manifest version {}",
                self.format_version
            )));
        }
        let session_id = parse_session_id(&self.session_id)?;
        CaptureStoreDescriptor::new(
            session_id,
            self.channels
                .iter()
                .cloned()
                .map(crate::CaptureChannelId::new)
                .collect::<Vec<_>>(),
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedSessionMetadata {
    format_version: u16,
    session_id: String,
    channels: Vec<String>,
    timeline: Option<PersistedTimeline>,
    outcome: CaptureSessionOutcome,
    created_unix_ns: u64,
    accessed_unix_ns: u64,
    recording_origin: Option<u64>,
    retained_start_sample: u64,
    kept: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedTimeline {
    sample_rate_hz: f64,
    channel_names: Vec<String>,
    trigger_sample: Option<u64>,
}

impl PersistedSessionMetadata {
    fn from_metadata(metadata: &CaptureSessionMetadata) -> Self {
        Self {
            format_version: 1,
            session_id: metadata.descriptor.session_id().to_string(),
            channels: metadata
                .descriptor
                .channels()
                .iter()
                .map(ToString::to_string)
                .collect(),
            timeline: metadata
                .timeline
                .as_ref()
                .map(|timeline| PersistedTimeline {
                    sample_rate_hz: timeline.sample_rate_hz(),
                    channel_names: timeline.channel_names().to_vec(),
                    trigger_sample: timeline.trigger_sample(),
                }),
            outcome: metadata.outcome,
            created_unix_ns: metadata.created_unix_ns,
            accessed_unix_ns: metadata.accessed_unix_ns,
            recording_origin: metadata.recording_origin,
            retained_start_sample: metadata.retained_start_sample,
            kept: metadata.kept,
        }
    }

    fn metadata(self) -> CaptureStoreResult<CaptureSessionMetadata> {
        if self.format_version != 1 {
            return Err(CaptureStoreError::Corrupt(format!(
                "unsupported capture session version {}",
                self.format_version
            )));
        }
        let descriptor = CaptureStoreDescriptor::new(
            parse_session_id(&self.session_id)?,
            self.channels
                .into_iter()
                .map(crate::CaptureChannelId::new)
                .collect::<Vec<_>>(),
        )?;
        let timeline = self
            .timeline
            .map(|timeline| {
                let mut metadata =
                    CaptureTimelineMetadata::new(timeline.sample_rate_hz, timeline.channel_names)?;
                metadata.set_trigger_sample(timeline.trigger_sample);
                Ok::<_, CaptureStoreError>(metadata)
            })
            .transpose()?;
        Ok(CaptureSessionMetadata {
            descriptor,
            timeline,
            outcome: self.outcome,
            created_unix_ns: self.created_unix_ns,
            accessed_unix_ns: self.accessed_unix_ns,
            recording_origin: self.recording_origin,
            retained_start_sample: self.retained_start_sample,
            kept: self.kept,
        })
    }
}

struct StoreState {
    manifest: PersistedManifest,
    writer_open: bool,
    writer_failure: Option<String>,
}

struct SharedStore {
    repository: Arc<dyn ArtifactRepository>,
    descriptor: CaptureStoreDescriptor,
    time_source: Arc<dyn UnixTimeSource>,
    state: Mutex<StoreState>,
    changed: Condvar,
}

impl SharedStore {
    fn snapshot(&self) -> CaptureStoreSnapshot {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        CaptureStoreSnapshot {
            committed_chunks: state.manifest.committed_chunks,
            committed_samples: state.manifest.committed_samples,
            committed_data_bytes: state.manifest.committed_data_bytes,
            writer_open: state.writer_open,
            writer_failed: state.writer_failure.is_some(),
            finalized: state.manifest.finalized,
            resident_commit_records: 0,
        }
    }

    fn publish_manifest(&self, manifest: &PersistedManifest) -> CaptureStoreResult<()> {
        publish_json(
            self.repository.as_ref(),
            manifest_key(self.descriptor.session_id())?,
            manifest,
        )
    }

    fn record_failure(&self, error: &CaptureStoreError) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.writer_failure = Some(error.to_string());
        self.changed.notify_all();
    }
}

#[derive(Clone)]
pub struct CaptureStore {
    shared: Arc<SharedStore>,
}

impl CaptureStore {
    /// Creates fresh persistent artifacts and a writer for one session.
    ///
    /// # Parameters
    /// - `config`: Repository, descriptor, and time source for the new session.
    pub fn create(config: CaptureStoreConfig) -> CaptureStoreResult<(Self, CaptureStoreWriter)> {
        let key = manifest_key(config.descriptor.session_id())?;
        if config.repository.open(&key)?.is_some() {
            return Err(CaptureStoreError::InvalidConfig(format!(
                "capture session {} already exists",
                config.descriptor.session_id()
            )));
        }
        let manifest = PersistedManifest {
            format_version: 1,
            session_id: config.descriptor.session_id().to_string(),
            channels: config
                .descriptor
                .channels()
                .iter()
                .map(ToString::to_string)
                .collect(),
            generation: 0,
            first_sequence: 0,
            committed_chunks: 0,
            committed_samples: 0,
            committed_data_bytes: 0,
            finalized: false,
        };
        publish_json(config.repository.as_ref(), key, &manifest)?;
        let now = config.time_source.now_unix_ns();
        write_session_metadata(
            config.repository.as_ref(),
            &CaptureSessionMetadata {
                descriptor: config.descriptor.clone(),
                timeline: None,
                outcome: CaptureSessionOutcome::InProgress,
                created_unix_ns: now,
                accessed_unix_ns: now,
                recording_origin: None,
                retained_start_sample: 0,
                kept: false,
            },
        )?;
        let shared = Arc::new(SharedStore {
            repository: config.repository,
            descriptor: config.descriptor,
            time_source: config.time_source,
            state: Mutex::new(StoreState {
                manifest,
                writer_open: true,
                writer_failure: None,
            }),
            changed: Condvar::new(),
        });
        Ok((
            Self {
                shared: Arc::clone(&shared),
            },
            CaptureStoreWriter {
                shared,
                next_sequence: 0,
                next_sample: 0,
                terminal: false,
            },
        ))
    }

    /// Opens an existing live or finalized session without recovery.
    ///
    /// # Parameters
    ///
    /// - `repository`: Artifact repository holding the session.
    /// - `session_id`: Session identity to open.
    pub fn open(
        repository: Arc<dyn ArtifactRepository>,
        session_id: CaptureSessionId,
    ) -> CaptureStoreResult<Self> {
        let manifest: PersistedManifest =
            read_json(repository.as_ref(), &manifest_key(session_id)?)?
                .ok_or(CaptureStoreError::SessionNotFound(session_id))?;
        let descriptor = manifest.descriptor()?;
        if descriptor.session_id() != session_id {
            return Err(CaptureStoreError::Corrupt(
                "capture manifest has the wrong session identity".into(),
            ));
        }
        Ok(Self {
            shared: Arc::new(SharedStore {
                repository,
                descriptor,
                time_source: Arc::new(SystemUnixTimeSource),
                state: Mutex::new(StoreState {
                    writer_open: false,
                    writer_failure: None,
                    manifest,
                }),
                changed: Condvar::new(),
            }),
        })
    }

    /// Returns this store's stable descriptor.
    pub fn descriptor(&self) -> &CaptureStoreDescriptor {
        &self.shared.descriptor
    }

    /// Returns the artifact repository used by this store.
    pub fn repository(&self) -> Arc<dyn ArtifactRepository> {
        Arc::clone(&self.shared.repository)
    }

    /// Returns the current monotonically increasing commit generation.
    pub fn generation(&self) -> u64 {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .manifest
            .generation
    }

    /// Returns a consistent summary of currently committed data and writer state.
    pub fn snapshot(&self) -> CaptureStoreSnapshot {
        self.shared.snapshot()
    }

    /// Opens a sequential cursor at the earliest retained chunk.
    pub fn open_cursor(&self) -> CaptureStoreResult<CaptureCursor> {
        let first_sequence = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .manifest
            .first_sequence;
        Ok(CaptureCursor {
            shared: Arc::clone(&self.shared),
            next_sequence: first_sequence,
        })
    }

    /// Opens a random-access reader over currently committed capture data.
    pub fn open_random_reader(&self) -> CaptureStoreResult<CaptureRandomReader> {
        Ok(CaptureRandomReader {
            shared: Arc::clone(&self.shared),
        })
    }

    /// Persists the requested capture session plan for later recovery and inspection.
    ///
    /// # Parameters
    /// - `plan`: Negotiated retention and completion plan to persist.
    pub fn write_session_plan(&self, plan: &CaptureSessionPlan) -> CaptureStoreResult<()> {
        publish_json(
            self.shared.repository.as_ref(),
            plan_key(self.descriptor().session_id())?,
            plan,
        )
    }

    /// Persists display timebase and channel metadata for later reopening.
    ///
    /// # Parameters
    ///
    /// - `timeline`: Validated presentation metadata in descriptor channel order.
    pub fn write_timeline_metadata(
        &self,
        timeline: CaptureTimelineMetadata,
    ) -> CaptureStoreResult<()> {
        let mut metadata = self.session_metadata()?.ok_or_else(|| {
            CaptureStoreError::Corrupt("capture session metadata is missing".into())
        })?;
        metadata.timeline = Some(timeline);
        metadata.accessed_unix_ns = self.shared.time_source.now_unix_ns();
        write_session_metadata(self.shared.repository.as_ref(), &metadata)
    }

    /// Reads the persisted session plan when it has been written.
    pub fn session_plan(&self) -> CaptureStoreResult<Option<CaptureSessionPlan>> {
        read_json(
            self.shared.repository.as_ref(),
            &plan_key(self.descriptor().session_id())?,
        )
    }

    /// Reads durable lifecycle metadata when it has been written.
    pub fn session_metadata(&self) -> CaptureStoreResult<Option<CaptureSessionMetadata>> {
        read_session_metadata(
            self.shared.repository.as_ref(),
            self.descriptor().session_id(),
        )
    }

    /// Finalizes the store as a normally completed capture.
    pub fn finalize(&self) -> CaptureStoreResult<FinalizedCapture> {
        self.finalize_with_outcome(CaptureSessionOutcome::Complete, None)
    }

    /// Finalizes the store with a terminal outcome and optional recording origin.
    ///
    /// # Parameters
    ///
    /// - `outcome`: Terminal capture outcome to persist.
    /// - `recording_origin`: Authoritative sample mapped to recording-time zero.
    pub fn finalize_with_outcome(
        &self,
        outcome: CaptureSessionOutcome,
        recording_origin: Option<u64>,
    ) -> CaptureStoreResult<FinalizedCapture> {
        self.finalize_with_details(outcome, recording_origin, None)
    }

    /// Finalizes with terminal state and optional timeline metadata details.
    pub fn finalize_with_details(
        &self,
        outcome: CaptureSessionOutcome,
        recording_origin: Option<u64>,
        trigger_sample: Option<u64>,
    ) -> CaptureStoreResult<FinalizedCapture> {
        if !outcome.is_terminal() {
            return Err(CaptureStoreError::InvalidConfig(
                "a finalized capture requires a terminal outcome".into(),
            ));
        }
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.writer_open {
            return Err(CaptureStoreError::WriterStillOpen);
        }
        if state.manifest.finalized {
            return Err(CaptureStoreError::AlreadyFinalized);
        }
        if let Some(failure) = &state.writer_failure {
            return Err(CaptureStoreError::WriterFailed(failure.clone()));
        }
        let mut manifest = state.manifest.clone();
        manifest.finalized = true;
        manifest.generation = manifest.generation.saturating_add(1);
        self.shared.publish_manifest(&manifest)?;
        state.manifest = manifest;
        drop(state);

        let mut metadata = self.session_metadata()?.ok_or_else(|| {
            CaptureStoreError::Corrupt("capture session metadata is missing".into())
        })?;
        metadata.outcome = outcome;
        metadata.recording_origin = recording_origin;
        metadata.accessed_unix_ns = self.shared.time_source.now_unix_ns();
        if let Some(timeline) = metadata.timeline.as_mut() {
            timeline.set_trigger_sample(trigger_sample);
        }
        write_session_metadata(self.shared.repository.as_ref(), &metadata)?;
        Ok(FinalizedCapture {
            store: self.clone(),
        })
    }

    fn manifest(&self) -> CaptureStoreManifest {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        CaptureStoreManifest {
            descriptor: self.shared.descriptor.clone(),
            committed_chunks: state.manifest.committed_chunks,
            committed_samples: state.manifest.committed_samples,
            committed_data_bytes: state.manifest.committed_data_bytes,
        }
    }
}

pub struct CaptureStoreWriter {
    shared: Arc<SharedStore>,
    next_sequence: u64,
    next_sample: u64,
    terminal: bool,
}

impl CaptureStoreWriter {
    fn append_inner(&mut self, chunk: CaptureChunk) -> CaptureStoreResult<()> {
        if self.terminal {
            return Err(CaptureStoreError::WriterFailed(
                "capture-store writer is already finished".into(),
            ));
        }
        if chunk.session_id() != self.shared.descriptor.session_id()
            || chunk.channels() != self.shared.descriptor.channels()
            || chunk.sequence() != self.next_sequence
            || chunk.start_sample() != self.next_sample
        {
            return Err(CaptureStoreError::InvalidChunk(
                "chunk does not continue the capture session".into(),
            ));
        }
        let encoded = encode_chunk(&chunk)?;
        publish_bytes(
            self.shared.repository.as_ref(),
            chunk_key(chunk.session_id(), chunk.sequence())?,
            &encoded,
        )?;
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut manifest = state.manifest.clone();
        manifest.committed_chunks = manifest
            .committed_chunks
            .checked_add(1)
            .ok_or_else(|| CaptureStoreError::InvalidChunk("chunk count overflow".into()))?;
        manifest.committed_samples = chunk.end_sample();
        manifest.committed_data_bytes = manifest
            .committed_data_bytes
            .checked_add(chunk.encoded_byte_len() as u64)
            .ok_or_else(|| CaptureStoreError::InvalidChunk("capture length overflow".into()))?;
        manifest.generation = manifest
            .generation
            .checked_add(1)
            .ok_or_else(|| CaptureStoreError::InvalidChunk("capture generation overflow".into()))?;
        self.shared.publish_manifest(&manifest)?;
        state.manifest = manifest;
        self.next_sequence += 1;
        self.next_sample = chunk.end_sample();
        self.shared.changed.notify_all();
        Ok(())
    }

    fn finish_inner(&mut self) {
        if !self.terminal {
            self.terminal = true;
            let mut state = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.writer_open = false;
            self.shared.changed.notify_all();
        }
    }
}

impl CaptureChunkWriter for CaptureStoreWriter {
    fn append(&mut self, chunk: CaptureChunk) -> Result<(), CaptureWriteError> {
        self.append_inner(chunk).map_err(|error| {
            self.shared.record_failure(&error);
            CaptureWriteError::Rejected(error.to_string())
        })
    }

    fn finish(&mut self) -> Result<(), CaptureWriteError> {
        self.finish_inner();
        Ok(())
    }
}

impl Drop for CaptureStoreWriter {
    fn drop(&mut self) {
        self.finish_inner();
    }
}

pub struct CaptureCursor {
    shared: Arc<SharedStore>,
    next_sequence: u64,
}

impl CaptureCursor {
    fn item(&mut self, wait: Option<Duration>) -> CaptureStoreResult<CaptureCursorItem> {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(timeout) = wait
            && self.next_sequence >= state.manifest.committed_chunks
            && state.writer_open
        {
            (state, _) = self
                .shared
                .changed
                .wait_timeout(state, timeout)
                .unwrap_or_else(|error| error.into_inner());
        }
        if self.next_sequence < state.manifest.first_sequence {
            self.next_sequence = state.manifest.first_sequence;
        }
        if self.next_sequence < state.manifest.committed_chunks {
            let sequence = self.next_sequence;
            drop(state);
            let chunk = read_chunk(
                self.shared.repository.as_ref(),
                &self.shared.descriptor,
                sequence,
            )?;
            self.next_sequence += 1;
            Ok(CaptureCursorItem::Chunk(chunk))
        } else if state.writer_open {
            Ok(CaptureCursorItem::Pending)
        } else {
            Ok(CaptureCursorItem::End)
        }
    }
}

impl CaptureStoreCursor for CaptureCursor {
    fn next(&mut self) -> CaptureStoreResult<CaptureCursorItem> {
        self.item(None)
    }

    fn wait_next(&mut self, timeout: Duration) -> CaptureStoreResult<CaptureCursorItem> {
        self.item(Some(timeout))
    }

    fn next_sequence(&self) -> u64 {
        self.next_sequence
    }
}

pub struct CaptureRandomReader {
    shared: Arc<SharedStore>,
}

impl CaptureRandomReader {
    /// Reads a bounded sampled window from the finalized capture artifact.
    ///
    /// # Parameters
    /// - `channels`: Indices into the descriptor channel table to sample.
    /// - `start_sample`: Inclusive authoritative sample bound.
    /// - `end_sample`: Exclusive authoritative sample bound.
    pub fn sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
    ) -> signal_capture::Result<CaptureSampledWindow> {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if start_sample >= end_sample || end_sample > state.manifest.committed_samples {
            return Err(Error::OutOfBounds(end_sample));
        }
        let first_sequence = state.manifest.first_sequence;
        let committed_chunks = state.manifest.committed_chunks;
        drop(state);
        if channels
            .iter()
            .any(|channel| *channel >= self.shared.descriptor.channels().len())
        {
            return Err(Error::InvalidProbe(
                *channels
                    .iter()
                    .find(|channel| **channel >= self.shared.descriptor.channels().len())
                    .expect("invalid channel exists"),
            ));
        }
        let mut sampled = channels
            .iter()
            .map(|channel| CaptureSampledChannel {
                channel: *channel,
                name: self.shared.descriptor.channels()[*channel].to_string(),
                initial: false,
                transitions: Vec::new(),
                waveform: Vec::new(),
            })
            .collect::<Vec<_>>();
        let mut current = vec![None; channels.len()];
        for sequence in first_sequence..committed_chunks {
            let chunk = read_chunk(
                self.shared.repository.as_ref(),
                &self.shared.descriptor,
                sequence,
            )
            .map_err(store_as_capture_error)?;
            if chunk.end_sample() <= start_sample {
                continue;
            }
            if chunk.start_sample() >= end_sample {
                break;
            }
            for sample in start_sample.max(chunk.start_sample())..end_sample.min(chunk.end_sample())
            {
                for (requested, channel) in channels.iter().copied().enumerate() {
                    let value = chunk
                        .packed_level(sample - chunk.start_sample(), channel)
                        .expect("validated chunk covers requested sample");
                    match current[requested] {
                        None => {
                            sampled[requested].initial = value;
                            current[requested] = Some(value);
                        }
                        Some(previous) if previous != value => {
                            sampled[requested]
                                .transitions
                                .push(CaptureTransition { sample, value });
                            current[requested] = Some(value);
                        }
                        Some(_) => {}
                    }
                }
            }
        }
        Ok(CaptureSampledWindow {
            start_sample,
            end_sample,
            sample_step: 1,
            channels: sampled,
        })
    }
}

#[derive(Clone)]
pub struct FinalizedCapture {
    store: CaptureStore,
}

impl FinalizedCapture {
    /// Opens a finalized session that is already consistent on disk.
    ///
    /// # Parameters
    /// - `repository`: Artifact repository holding the session.
    /// - `session_id`: Finalized session identity to open.
    pub fn open(
        repository: Arc<dyn ArtifactRepository>,
        session_id: CaptureSessionId,
    ) -> CaptureStoreResult<Self> {
        let store = CaptureStore::open(repository, session_id)?;
        if !store.snapshot().finalized {
            return Err(CaptureStoreError::NotFinalized);
        }
        Ok(Self { store })
    }

    /// Recovers an interrupted finalized session and reports repairs.
    ///
    /// # Parameters
    ///
    /// - `repository`: Artifact repository holding the session.
    /// - `session_id`: Session identity to recover and open.
    pub fn recover(
        repository: Arc<dyn ArtifactRepository>,
        session_id: CaptureSessionId,
    ) -> CaptureStoreResult<(Self, CaptureRecoveryReport)> {
        let store = CaptureStore::open(repository, session_id)?;
        if !store.snapshot().finalized {
            let mut metadata = store.session_metadata()?.ok_or_else(|| {
                CaptureStoreError::Corrupt("capture session metadata is missing".into())
            })?;
            metadata.outcome = CaptureSessionOutcome::Incomplete;
            metadata.accessed_unix_ns = store.shared.time_source.now_unix_ns();
            write_session_metadata(store.shared.repository.as_ref(), &metadata)?;
            let mut state = store
                .shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut manifest = state.manifest.clone();
            manifest.finalized = true;
            manifest.generation = manifest.generation.saturating_add(1);
            store.shared.publish_manifest(&manifest)?;
            state.manifest = manifest;
        }
        Ok((Self { store }, CaptureRecoveryReport::default()))
    }

    /// Returns the durable capture manifest.
    pub fn manifest(&self) -> CaptureStoreManifest {
        self.store.manifest()
    }

    /// Returns the latest durable commit generation.
    pub fn generation(&self) -> u64 {
        self.store.generation()
    }

    /// Returns the artifact repository holding the capture.
    pub fn repository(&self) -> Arc<dyn ArtifactRepository> {
        self.store.repository()
    }

    /// Opens a sequential cursor over retained committed chunks.
    pub fn open_cursor(&self) -> CaptureStoreResult<CaptureCursor> {
        self.store.open_cursor()
    }

    /// Opens a random-access reader over retained committed data.
    pub fn open_random_reader(&self) -> CaptureStoreResult<CaptureRandomReader> {
        self.store.open_random_reader()
    }

    /// Reads the persisted negotiated capture plan, when present.
    pub fn session_plan(&self) -> CaptureStoreResult<Option<CaptureSessionPlan>> {
        self.store.session_plan()
    }

    /// Reads the persisted lifecycle metadata, when present.
    pub fn session_metadata(&self) -> CaptureStoreResult<Option<CaptureSessionMetadata>> {
        self.store.session_metadata()
    }

    /// Returns a cloneable handle to the underlying capture store.
    pub fn store_handle(&self) -> CaptureStore {
        self.store.clone()
    }

    /// Sets kept.
    ///
    /// # Parameters
    /// - `kept`: Whether automatic retention cleanup must preserve this session.
    pub fn set_kept(&self, kept: bool) -> CaptureStoreResult<()> {
        let mut metadata = self.session_metadata()?.ok_or_else(|| {
            CaptureStoreError::Corrupt("capture session metadata is missing".into())
        })?;
        metadata.kept = kept;
        metadata.accessed_unix_ns = self.store.shared.time_source.now_unix_ns();
        write_session_metadata(self.store.shared.repository.as_ref(), &metadata)
    }

    /// Updates the session's durable last-access timestamp.
    pub fn touch(&self) -> CaptureStoreResult<()> {
        let mut metadata = self.session_metadata()?.ok_or_else(|| {
            CaptureStoreError::Corrupt("capture session metadata is missing".into())
        })?;
        metadata.accessed_unix_ns = self.store.shared.time_source.now_unix_ns();
        write_session_metadata(self.store.shared.repository.as_ref(), &metadata)
    }

    /// Reclaims chunks wholly before a policy-approved sample boundary.
    ///
    /// # Parameters
    ///
    /// - `safe_sample`: First sample that must remain available after reclamation.
    pub fn reclaim_before(&self, safe_sample: u64) -> CaptureStoreResult<CaptureReclamationReport> {
        let mut state = self
            .store
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut sequence = state.manifest.first_sequence;
        let mut report = CaptureReclamationReport::default();
        while sequence < state.manifest.committed_chunks {
            let chunk = read_chunk(
                self.store.shared.repository.as_ref(),
                &self.store.shared.descriptor,
                sequence,
            )?;
            if chunk.end_sample() > safe_sample {
                break;
            }
            self.store
                .shared
                .repository
                .remove(&chunk_key(self.store.descriptor().session_id(), sequence)?)?;
            report.reclaimed_chunks += 1;
            report.reclaimed_samples += chunk.sample_count();
            report.reclaimed_data_bytes += chunk.encoded_byte_len() as u64;
            sequence += 1;
        }
        if report.reclaimed_chunks > 0 {
            let mut manifest = state.manifest.clone();
            manifest.first_sequence = sequence;
            manifest.generation = manifest.generation.saturating_add(1);
            self.store.shared.publish_manifest(&manifest)?;
            state.manifest = manifest;
            drop(state);
            let mut metadata = self.session_metadata()?.ok_or_else(|| {
                CaptureStoreError::Corrupt("capture session metadata is missing".into())
            })?;
            metadata.retained_start_sample = safe_sample;
            write_session_metadata(self.store.shared.repository.as_ref(), &metadata)?;
        }
        Ok(report)
    }
}

pub(crate) fn discover_sessions(
    repository: &dyn ArtifactRepository,
) -> CaptureStoreResult<Vec<(CaptureSessionId, u64)>> {
    let namespace = session_manifest_namespace()?;
    let mut sessions = Vec::new();
    for entry in repository.entries(&namespace)? {
        let manifest: PersistedManifest = read_json(repository, &entry.key)?.ok_or_else(|| {
            CaptureStoreError::Corrupt("listed capture manifest disappeared".into())
        })?;
        let descriptor = manifest.descriptor()?;
        sessions.push((descriptor.session_id(), manifest.committed_data_bytes));
    }
    Ok(sessions)
}

pub(crate) fn remove_session_artifacts(
    repository: &dyn ArtifactRepository,
    session_id: CaptureSessionId,
) -> CaptureStoreResult<()> {
    let manifest: PersistedManifest = read_json(repository, &manifest_key(session_id)?)?
        .ok_or(CaptureStoreError::SessionNotFound(session_id))?;
    for sequence in manifest.first_sequence..manifest.committed_chunks {
        repository.remove(&chunk_key(session_id, sequence)?)?;
    }
    repository.remove(&plan_key(session_id)?)?;
    repository.remove(&session_key(session_id)?)?;
    repository.remove(&manifest_key(session_id)?)?;
    Ok(())
}

fn session_manifest_namespace() -> CaptureStoreResult<ArtifactNamespace> {
    ArtifactNamespace::new(CAPTURE_MANIFEST_NAMESPACE).map_err(Into::into)
}

fn manifest_key(session_id: CaptureSessionId) -> CaptureStoreResult<ArtifactKey> {
    key(CAPTURE_MANIFEST_NAMESPACE, session_id, None)
}

fn chunk_key(session_id: CaptureSessionId, sequence: u64) -> CaptureStoreResult<ArtifactKey> {
    key(CAPTURE_CHUNK_NAMESPACE, session_id, Some(sequence))
}

fn session_key(session_id: CaptureSessionId) -> CaptureStoreResult<ArtifactKey> {
    key(CAPTURE_SESSION_NAMESPACE, session_id, None)
}

fn plan_key(session_id: CaptureSessionId) -> CaptureStoreResult<ArtifactKey> {
    key(CAPTURE_PLAN_NAMESPACE, session_id, None)
}

fn key(
    namespace: &str,
    session_id: CaptureSessionId,
    sequence: Option<u64>,
) -> CaptureStoreResult<ArtifactKey> {
    let namespace = ArtifactNamespace::new(namespace)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(namespace.as_str().as_bytes());
    hasher.update(&session_id.get().to_le_bytes());
    if let Some(sequence) = sequence {
        hasher.update(&sequence.to_le_bytes());
    }
    Ok(ArtifactKey::new(
        namespace,
        SourceIdentity::from_bytes(*hasher.finalize().as_bytes()),
    ))
}

fn encode_chunk(chunk: &CaptureChunk) -> CaptureStoreResult<Vec<u8>> {
    let CaptureChunkPayload::PackedLsbFirst { bytes, bit_offset } = chunk.payload();
    let channel_bytes = chunk
        .channels()
        .iter()
        .try_fold(Vec::new(), |mut output, channel| {
            let length = u32::try_from(channel.as_str().len()).map_err(|_| {
                CaptureStoreError::InvalidChunk("capture channel identity is too long".into())
            })?;
            output.extend_from_slice(&length.to_le_bytes());
            output.extend_from_slice(channel.as_str().as_bytes());
            Ok::<_, CaptureStoreError>(output)
        })?;
    let channel_count = u32::try_from(chunk.channels().len())
        .map_err(|_| CaptureStoreError::InvalidChunk("too many capture channels".into()))?;
    let payload_len = bytes.len() as u64;
    let mut output = Vec::with_capacity(64 + channel_bytes.len() + bytes.len());
    output.extend_from_slice(CHUNK_MAGIC);
    output.extend_from_slice(&CHUNK_FORMAT_VERSION.to_le_bytes());
    output.push(*bit_offset);
    output.push(0);
    output.extend_from_slice(&chunk.session_id().get().to_le_bytes());
    output.extend_from_slice(&chunk.sequence().to_le_bytes());
    output.extend_from_slice(&chunk.start_sample().to_le_bytes());
    output.extend_from_slice(&chunk.sample_count().to_le_bytes());
    output.extend_from_slice(&channel_count.to_le_bytes());
    output.extend_from_slice(&payload_len.to_le_bytes());
    output.extend_from_slice(&channel_bytes);
    output.extend_from_slice(bytes.as_slice());
    let checksum = platform_artifacts::checksum_parts(&[&output]);
    output.extend_from_slice(&checksum.to_le_bytes());
    Ok(output)
}

fn read_chunk(
    repository: &dyn ArtifactRepository,
    descriptor: &CaptureStoreDescriptor,
    sequence: u64,
) -> CaptureStoreResult<CaptureChunk> {
    let key = chunk_key(descriptor.session_id(), sequence)?;
    let mut reader = repository.open(&key)?.ok_or_else(|| {
        CaptureStoreError::Corrupt(format!("capture chunk {sequence} is missing"))
    })?;
    let length = reader.len()?;
    let range = ByteRange::new(0, length).map_err(RepositoryError::from)?;
    let backing = read_artifact_region(reader.as_mut(), range)?;
    let region = ByteRegion::new(Arc::clone(&backing), range).map_err(RepositoryError::from)?;
    let bytes = region.bytes();
    if bytes.len() < 68 || &bytes[..8] != CHUNK_MAGIC {
        return Err(CaptureStoreError::Corrupt(format!(
            "capture chunk {sequence} has an invalid header"
        )));
    }
    let stored_checksum = get_u32(bytes, bytes.len() - 4)?;
    if platform_artifacts::checksum_parts(&[&bytes[..bytes.len() - 4]]) != stored_checksum {
        return Err(CaptureStoreError::Corrupt(format!(
            "capture chunk {sequence} checksum mismatch"
        )));
    }
    if get_u16(bytes, 8)? != CHUNK_FORMAT_VERSION
        || get_u128(bytes, 12)? != descriptor.session_id().get()
        || get_u64(bytes, 28)? != sequence
    {
        return Err(CaptureStoreError::Corrupt(format!(
            "capture chunk {sequence} identity mismatch"
        )));
    }
    let bit_offset = bytes[10];
    let start_sample = get_u64(bytes, 36)?;
    let sample_count = get_u64(bytes, 44)?;
    let channel_count = usize::try_from(get_u32(bytes, 52)?)
        .map_err(|_| CaptureStoreError::Corrupt("capture channel count is too large".into()))?;
    let payload_len = get_u64(bytes, 56)?;
    let mut offset = 64_usize;
    let mut channels = Vec::with_capacity(channel_count);
    for _ in 0..channel_count {
        let name_len = usize::try_from(get_u32(bytes, offset)?)
            .map_err(|_| CaptureStoreError::Corrupt("capture channel name is too large".into()))?;
        offset = offset
            .checked_add(4)
            .ok_or_else(|| CaptureStoreError::Corrupt("chunk channel offset overflow".into()))?;
        let end = offset
            .checked_add(name_len)
            .ok_or_else(|| CaptureStoreError::Corrupt("chunk channel offset overflow".into()))?;
        let name = std::str::from_utf8(bytes.get(offset..end).ok_or_else(|| {
            CaptureStoreError::Corrupt("capture chunk channel table is truncated".into())
        })?)
        .map_err(|_| CaptureStoreError::Corrupt("capture channel identity is not UTF-8".into()))?;
        channels.push(crate::CaptureChannelId::new(name));
        offset = end;
    }
    if channels.as_slice() != descriptor.channels() {
        return Err(CaptureStoreError::Corrupt(
            "capture chunk channel table differs from its manifest".into(),
        ));
    }
    let payload_end =
        offset
            .checked_add(usize::try_from(payload_len).map_err(|_| {
                CaptureStoreError::Corrupt("capture chunk payload is too large".into())
            })?)
            .ok_or_else(|| CaptureStoreError::Corrupt("chunk payload offset overflow".into()))?;
    if payload_end + 4 != bytes.len() {
        return Err(CaptureStoreError::Corrupt(
            "capture chunk payload length is invalid".into(),
        ));
    }
    let payload_offset = u64::try_from(offset)
        .map_err(|_| CaptureStoreError::Corrupt("capture payload offset exceeds u64".into()))?;
    let payload_range =
        ByteRange::new(payload_offset, payload_len).map_err(RepositoryError::from)?;
    let payload = ByteRegion::new(backing, payload_range).map_err(RepositoryError::from)?;
    CaptureChunk::packed_lsb_first(
        descriptor.session_id(),
        sequence,
        start_sample,
        sample_count,
        descriptor.channel_table(),
        payload,
        bit_offset,
    )
    .map_err(|error| CaptureStoreError::Corrupt(error.to_string()))
}

fn publish_bytes(
    repository: &dyn ArtifactRepository,
    key: ArtifactKey,
    bytes: &[u8],
) -> CaptureStoreResult<()> {
    let mut writer = repository.begin_write(key)?;
    writer.write_at(0, bytes)?;
    writer.truncate(bytes.len() as u64)?;
    writer.flush()?;
    writer.publish()?;
    Ok(())
}

fn publish_json<T: Serialize>(
    repository: &dyn ArtifactRepository,
    key: ArtifactKey,
    value: &T,
) -> CaptureStoreResult<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| CaptureStoreError::InvalidConfig(error.to_string()))?;
    publish_bytes(repository, key, &bytes)
}

fn read_json<T: for<'de> Deserialize<'de>>(
    repository: &dyn ArtifactRepository,
    key: &ArtifactKey,
) -> CaptureStoreResult<Option<T>> {
    let Some(mut reader) = repository.open(key)? else {
        return Ok(None);
    };
    let length = reader.len()?;
    let resident = usize::try_from(length)
        .map_err(|_| CaptureStoreError::Corrupt("metadata artifact is too large".into()))?;
    let mut bytes = vec![0_u8; resident];
    let mut copied = 0;
    while copied < bytes.len() {
        let count = reader.read_at(copied as u64, &mut bytes[copied..])?;
        if count == 0 {
            return Err(CaptureStoreError::Corrupt(
                "metadata artifact is truncated".into(),
            ));
        }
        copied += count;
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| CaptureStoreError::Corrupt(error.to_string()))
}

fn write_session_metadata(
    repository: &dyn ArtifactRepository,
    metadata: &CaptureSessionMetadata,
) -> CaptureStoreResult<()> {
    publish_json(
        repository,
        session_key(metadata.descriptor.session_id())?,
        &PersistedSessionMetadata::from_metadata(metadata),
    )
}

fn read_session_metadata(
    repository: &dyn ArtifactRepository,
    session_id: CaptureSessionId,
) -> CaptureStoreResult<Option<CaptureSessionMetadata>> {
    read_json::<PersistedSessionMetadata>(repository, &session_key(session_id)?)?
        .map(PersistedSessionMetadata::metadata)
        .transpose()
}

fn parse_session_id(value: &str) -> CaptureStoreResult<CaptureSessionId> {
    u128::from_str_radix(value, 16)
        .map(CaptureSessionId::new)
        .map_err(|_| CaptureStoreError::Corrupt("capture session identity is invalid".into()))
}

fn store_as_capture_error(error: CaptureStoreError) -> Error {
    Error::ParseError(error.to_string())
}

fn get_u16(bytes: &[u8], offset: usize) -> CaptureStoreResult<u16> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| CaptureStoreError::Corrupt("capture chunk is truncated".into()))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn get_u32(bytes: &[u8], offset: usize) -> CaptureStoreResult<u32> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| CaptureStoreError::Corrupt("capture chunk is truncated".into()))?;
    Ok(u32::from_le_bytes(value.try_into().expect("four bytes")))
}

fn get_u64(bytes: &[u8], offset: usize) -> CaptureStoreResult<u64> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| CaptureStoreError::Corrupt("capture chunk is truncated".into()))?;
    Ok(u64::from_le_bytes(value.try_into().expect("eight bytes")))
}

fn get_u128(bytes: &[u8], offset: usize) -> CaptureStoreResult<u128> {
    let value = bytes
        .get(offset..offset + 16)
        .ok_or_else(|| CaptureStoreError::Corrupt("capture chunk is truncated".into()))?;
    Ok(u128::from_le_bytes(
        value.try_into().expect("sixteen bytes"),
    ))
}

#[cfg(test)]
mod artifact_store_tests {
    use std::sync::Arc;

    use platform_artifacts::{ArtifactRepository, MemoryArtifactRepository};

    use super::{CaptureStore, CaptureStoreConfig, FinalizedCapture, PersistedManifest, chunk_key};
    use crate::{
        CaptureChannelId, CaptureChunk, CaptureChunkWriter, CaptureSessionId, CaptureStoreCursor,
        CaptureStoreDescriptor, CaptureStoreError,
    };

    #[test]
    fn manifest_preserves_counts_above_the_wasm32_address_range() {
        let count = u64::from(u32::MAX) + 67;
        let manifest = PersistedManifest {
            format_version: 1,
            session_id: "17".into(),
            channels: vec!["data".into()],
            generation: count,
            first_sequence: count + 1,
            committed_chunks: count + 2,
            committed_samples: count + 3,
            committed_data_bytes: count + 4,
            finalized: true,
        };

        let encoded = serde_json::to_vec(&manifest).unwrap();
        let decoded: PersistedManifest = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded.generation, count);
        assert_eq!(decoded.committed_chunks, count + 2);
        assert_eq!(decoded.committed_samples, count + 3);
        assert_eq!(decoded.committed_data_bytes, count + 4);
    }

    #[test]
    fn memory_repository_supports_live_visibility_and_finalized_replay() {
        let repository = Arc::new(MemoryArtifactRepository::new());
        let descriptor = CaptureStoreDescriptor::new(
            CaptureSessionId::new(17),
            [
                CaptureChannelId::new("clock"),
                CaptureChannelId::new("data"),
            ],
        )
        .unwrap();
        let (store, mut writer) = CaptureStore::create(CaptureStoreConfig::new(
            repository.clone(),
            descriptor.clone(),
        ))
        .unwrap();
        writer
            .append(
                CaptureChunk::packed_lsb_first(
                    descriptor.session_id(),
                    0,
                    0,
                    4,
                    descriptor.channel_table(),
                    [0b0110_1001],
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        let mut cursor = store.open_cursor().unwrap();
        assert!(matches!(
            cursor.next().unwrap(),
            crate::CaptureCursorItem::Chunk(_)
        ));
        writer.finish().unwrap();
        let finalized = store.finalize().unwrap();
        assert_eq!(finalized.generation(), 2);

        let reopened = FinalizedCapture::open(repository, descriptor.session_id()).unwrap();
        let mut cursor = reopened.open_cursor().unwrap();
        let crate::CaptureCursorItem::Chunk(chunk) = cursor.next().unwrap() else {
            panic!("expected replayed chunk");
        };
        assert_eq!(chunk.packed_level(0, 0), Some(true));
        assert_eq!(chunk.packed_level(0, 1), Some(false));
        assert_eq!(cursor.next().unwrap(), crate::CaptureCursorItem::End);
    }

    #[test]
    fn corrupt_committed_chunk_is_rejected_without_exposing_payload_data() {
        let repository = Arc::new(MemoryArtifactRepository::new());
        let descriptor =
            CaptureStoreDescriptor::new(CaptureSessionId::new(18), [CaptureChannelId::new("data")])
                .unwrap();
        let (store, mut writer) = CaptureStore::create(CaptureStoreConfig::new(
            repository.clone(),
            descriptor.clone(),
        ))
        .unwrap();
        writer
            .append(
                CaptureChunk::packed_lsb_first(
                    descriptor.session_id(),
                    0,
                    0,
                    8,
                    descriptor.channel_table(),
                    [0xa5],
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        writer.finish().unwrap();
        store.finalize().unwrap();

        let key = chunk_key(descriptor.session_id(), 0).unwrap();
        let mut replacement = repository.begin_write(key).unwrap();
        replacement.write_at(0, b"corrupt").unwrap();
        replacement.publish().unwrap();

        let reopened = FinalizedCapture::open(repository, descriptor.session_id()).unwrap();
        let error = reopened.open_cursor().unwrap().next().unwrap_err();
        assert!(matches!(error, CaptureStoreError::Corrupt(_)));
    }
}
