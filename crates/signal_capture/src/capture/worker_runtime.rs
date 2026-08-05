use std::collections::BTreeMap;
use std::sync::Arc;

use signal_artifacts::{ArtifactRepository, SourceIdentity};
use signal_runtime::{WorkExecutor, WorkerOperation};

use super::host_protocol::{
    CaptureWorkerMessage, CaptureWorkerReplayBlock, CaptureWorkerReplayRequest,
    CaptureWorkerRequest,
};
use super::implementation::{
    CaptureIndex, CaptureIndexFactory, CaptureIndexOpenStep, CaptureIndexOpenTask,
};

type PreparationHandler =
    dyn Fn(Vec<u8>) -> Result<CaptureWorkerPreparedIndex, String> + Send + Sync + 'static;

/// Concrete index factory produced by one registered capture-worker operation.
pub struct CaptureWorkerPreparedIndex {
    source_identity: SourceIdentity,
    factory: Box<dyn CaptureIndexFactory>,
}

impl CaptureWorkerPreparedIndex {
    /// Associates a prepared index factory with its stable source identity.
    ///
    /// # Parameters
    /// - `source_identity`: Identity used to deduplicate preparation work.
    /// - `factory`: Factory that opens indexes for this capture source.
    pub fn new(source_identity: SourceIdentity, factory: Box<dyn CaptureIndexFactory>) -> Self {
        Self {
            source_identity,
            factory,
        }
    }

    fn into_parts(self) -> (SourceIdentity, Box<dyn CaptureIndexFactory>) {
        (self.source_identity, self.factory)
    }
}

/// Registry of stateful capture preparation operations available in one host.
#[derive(Default)]
pub struct CaptureWorkerOperationRegistry {
    handlers: BTreeMap<WorkerOperation, Arc<PreparationHandler>>,
}

impl CaptureWorkerOperationRegistry {
    /// Creates an empty registry of host preparation operations.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers one named capture preparation operation.
    ///
    /// # Parameters
    ///
    /// - `operation`: Provider-specific operation identifier.
    /// - `handler`: Decoder that turns opaque request bytes into an index factory.
    pub fn register(
        &mut self,
        operation: WorkerOperation,
        handler: impl Fn(Vec<u8>) -> Result<CaptureWorkerPreparedIndex, String> + Send + Sync + 'static,
    ) -> Result<(), String> {
        if self.handlers.contains_key(&operation) {
            return Err(format!(
                "capture-worker operation '{}' is already registered",
                operation.as_str()
            ));
        }
        self.handlers.insert(operation, Arc::new(handler));
        Ok(())
    }

    fn prepare(
        &self,
        operation: &WorkerOperation,
        payload: Vec<u8>,
    ) -> Result<CaptureWorkerPreparedIndex, String> {
        self.handlers
            .get(operation)
            .ok_or_else(|| {
                format!(
                    "capture-worker operation '{}' is not registered",
                    operation.as_str()
                )
            })?
            .as_ref()(payload)
    }
}

struct PreparedSession {
    display_name: String,
    source_identity: SourceIdentity,
    index_identity: SourceIdentity,
    metadata: super::implementation::CaptureMetadata,
    index: Box<dyn CaptureIndex + Send>,
    leases: u64,
}

struct PendingPreparation {
    sequences: Vec<u64>,
    display_name: String,
    source_identity: SourceIdentity,
    metadata: super::implementation::CaptureMetadata,
    task: Box<dyn CaptureIndexOpenTask>,
}

/// Stateful host runtime for prepared capture indexes and bounded queries.
pub struct CaptureWorkerRuntime {
    operations: CaptureWorkerOperationRegistry,
    artifact_repository: Arc<dyn ArtifactRepository>,
    work_executor: Arc<dyn WorkExecutor>,
    preparations: BTreeMap<u64, PendingPreparation>,
    preparations_by_identity: BTreeMap<SourceIdentity, u64>,
    sessions: BTreeMap<u64, PreparedSession>,
    sessions_by_identity: BTreeMap<SourceIdentity, u64>,
    next_session_id: u64,
}

impl CaptureWorkerRuntime {
    /// Creates a stateful runtime for host-worker capture preparation and queries.
    ///
    /// # Parameters
    /// - `operations`: Registered provider preparation handlers.
    /// - `artifact_repository`: Repository that owns generated index artifacts.
    /// - `work_executor`: Host capability used for bounded index construction.
    pub fn new(
        operations: CaptureWorkerOperationRegistry,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        Self {
            operations,
            artifact_repository,
            work_executor,
            preparations: BTreeMap::new(),
            preparations_by_identity: BTreeMap::new(),
            sessions: BTreeMap::new(),
            sessions_by_identity: BTreeMap::new(),
            next_session_id: 0,
        }
    }

    /// Executes a complete protocol request and collects immediately available messages.
    pub fn execute(&mut self, request: CaptureWorkerRequest) -> Vec<CaptureWorkerMessage> {
        let complete_preparation = matches!(request, CaptureWorkerRequest::Prepare { .. });
        let mut messages = Vec::new();
        self.execute_streaming(request, &mut |message| messages.push(message));
        while complete_preparation && self.advance_streaming(&mut |message| messages.push(message))
        {
        }
        messages
    }

    /// Executes one request and publishes each result as soon as it is available.
    pub fn execute_streaming(
        &mut self,
        request: CaptureWorkerRequest,
        emit: &mut dyn FnMut(CaptureWorkerMessage),
    ) {
        match request {
            CaptureWorkerRequest::Prepare { sequence, request } => {
                self.prepare(sequence, request.into_parts(), emit);
            }
            CaptureWorkerRequest::Query {
                sequence,
                session_id,
                query,
            } => emit(self.query(sequence, session_id, query)),
            CaptureWorkerRequest::Replay {
                sequence,
                session_id,
                request,
            } => emit(self.replay(sequence, session_id, request)),
            CaptureWorkerRequest::Cancel { sequence } => self.cancel(sequence, emit),
            CaptureWorkerRequest::Release { session_id } => {
                self.release(session_id);
            }
        }
    }

    /// Advances at most one deterministic capture-index work unit.
    ///
    /// Returns whether another preparation remains runnable.
    pub fn advance_streaming(&mut self, emit: &mut dyn FnMut(CaptureWorkerMessage)) -> bool {
        let Some(sequence) = self.preparations.keys().next().copied() else {
            return false;
        };
        let Some(mut preparation) = self.preparations.remove(&sequence) else {
            return false;
        };
        match preparation.task.step() {
            Ok(CaptureIndexOpenStep::Progress(progress)) => {
                for sequence in &preparation.sequences {
                    emit(CaptureWorkerMessage::Progress {
                        sequence: *sequence,
                        progress,
                    });
                }
                self.preparations.insert(sequence, preparation);
            }
            Ok(CaptureIndexOpenStep::Ready(index)) => {
                self.preparations_by_identity
                    .remove(&preparation.source_identity);
                self.complete_preparation(preparation, index, emit);
            }
            Err(error) => {
                self.preparations_by_identity
                    .remove(&preparation.source_identity);
                for sequence in preparation.sequences {
                    emit(CaptureWorkerMessage::Failed {
                        sequence,
                        message: error.to_string(),
                    });
                }
            }
        }
        !self.preparations.is_empty()
    }

    /// Returns whether pending preparations.
    pub fn has_pending_preparations(&self) -> bool {
        !self.preparations.is_empty()
    }

    fn prepare(
        &mut self,
        sequence: u64,
        (operation, payload): (WorkerOperation, Vec<u8>),
        emit: &mut dyn FnMut(CaptureWorkerMessage),
    ) {
        let prepared = match self.operations.prepare(&operation, payload) {
            Ok(prepared) => prepared,
            Err(message) => {
                emit(CaptureWorkerMessage::Failed { sequence, message });
                return;
            }
        };
        let (source_identity, factory) = prepared.into_parts();
        if let Some(session_id) = self.sessions_by_identity.get(&source_identity).copied()
            && let Some(session) = self.sessions.get_mut(&session_id)
        {
            session.leases = session.leases.saturating_add(1);
            emit(CaptureWorkerMessage::Metadata {
                sequence,
                metadata: session.metadata.clone(),
            });
            emit(CaptureWorkerMessage::Prepared {
                sequence,
                session_id,
                display_name: session.display_name.clone(),
                source_identity: session.source_identity,
                index_identity: session.index_identity,
                metadata: session.metadata.clone(),
            });
            return;
        }
        if let Some(primary_sequence) = self.preparations_by_identity.get(&source_identity).copied()
            && let Some(preparation) = self.preparations.get_mut(&primary_sequence)
        {
            preparation.sequences.push(sequence);
            emit(CaptureWorkerMessage::Metadata {
                sequence,
                metadata: preparation.metadata.clone(),
            });
            return;
        }
        let display_name = factory.display_name();
        let metadata = match factory.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                emit(CaptureWorkerMessage::Failed {
                    sequence,
                    message: error.to_string(),
                });
                return;
            }
        };
        emit(CaptureWorkerMessage::Metadata {
            sequence,
            metadata: metadata.clone(),
        });
        let task = match factory.open_task(
            Arc::clone(&self.artifact_repository),
            Arc::clone(&self.work_executor),
        ) {
            Ok(task) => task,
            Err(error) => {
                emit(CaptureWorkerMessage::Failed {
                    sequence,
                    message: error.to_string(),
                });
                return;
            }
        };
        self.preparations.insert(
            sequence,
            PendingPreparation {
                sequences: vec![sequence],
                display_name,
                source_identity,
                metadata,
                task,
            },
        );
        self.preparations_by_identity
            .insert(source_identity, sequence);
    }

    fn complete_preparation(
        &mut self,
        preparation: PendingPreparation,
        index: Box<dyn CaptureIndex + Send>,
        emit: &mut dyn FnMut(CaptureWorkerMessage),
    ) {
        let PendingPreparation {
            sequences,
            display_name,
            source_identity,
            metadata,
            task,
        } = preparation;
        let index_identity = index.index_identity();
        if task
            .expected_index_identity()
            .is_some_and(|expected| expected != index_identity)
        {
            for sequence in sequences {
                emit(CaptureWorkerMessage::Failed {
                    sequence,
                    message:
                        "prepared capture index does not match the identity declared by its builder"
                            .to_owned(),
                });
            }
            return;
        }
        self.next_session_id = match self.next_session_id.checked_add(1) {
            Some(session_id) => session_id,
            None => {
                for sequence in sequences {
                    emit(CaptureWorkerMessage::Failed {
                        sequence,
                        message: "capture-worker session identifiers are exhausted".to_owned(),
                    });
                }
                return;
            }
        };
        let session_id = self.next_session_id;
        let leases = sequences.len() as u64;
        self.sessions.insert(
            session_id,
            PreparedSession {
                display_name: display_name.clone(),
                source_identity,
                index_identity,
                metadata: metadata.clone(),
                index,
                leases,
            },
        );
        self.sessions_by_identity
            .insert(source_identity, session_id);
        for sequence in sequences {
            emit(CaptureWorkerMessage::Prepared {
                sequence,
                session_id,
                display_name: display_name.clone(),
                source_identity,
                index_identity,
                metadata: metadata.clone(),
            });
        }
    }

    fn cancel(&mut self, sequence: u64, emit: &mut dyn FnMut(CaptureWorkerMessage)) {
        let primary = self.preparations.iter().find_map(|(primary, preparation)| {
            preparation
                .sequences
                .contains(&sequence)
                .then_some(*primary)
        });
        let Some(primary) = primary else {
            return;
        };
        let remove_preparation = {
            let preparation = self
                .preparations
                .get_mut(&primary)
                .expect("preparation was found above");
            preparation
                .sequences
                .retain(|candidate| *candidate != sequence);
            preparation.sequences.is_empty()
        };
        emit(CaptureWorkerMessage::Cancelled { sequence });
        if remove_preparation && let Some(preparation) = self.preparations.remove(&primary) {
            self.preparations_by_identity
                .remove(&preparation.source_identity);
        }
    }

    fn release(&mut self, session_id: u64) {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if session.leases > 1 {
            session.leases -= 1;
            return;
        }
        let source_identity = session.source_identity;
        self.sessions.remove(&session_id);
        self.sessions_by_identity.remove(&source_identity);
    }

    fn query(
        &mut self,
        sequence: u64,
        session_id: u64,
        query: super::query::CaptureIndexQuery,
    ) -> CaptureWorkerMessage {
        let result = (|| {
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| format!("capture-worker session {session_id} does not exist"))?;
            let channels = query
                .channels
                .into_iter()
                .map(|channel| {
                    usize::try_from(channel)
                        .map_err(|_| format!("capture channel {channel} exceeds this host"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let target_points = usize::try_from(query.target_points)
                .map_err(|_| "capture query point limit exceeds this host".to_owned())?;
            session
                .index
                .sampled_window(
                    &channels,
                    query.start_sample,
                    query.end_sample,
                    target_points,
                )
                .map_err(|error| error.to_string())
        })();
        match result {
            Ok(window) => CaptureWorkerMessage::Window { sequence, window },
            Err(message) => CaptureWorkerMessage::Failed { sequence, message },
        }
    }

    fn replay(
        &mut self,
        sequence: u64,
        session_id: u64,
        request: CaptureWorkerReplayRequest,
    ) -> CaptureWorkerMessage {
        let block = request.block;
        let result = (|| {
            if request.max_payload_bytes == 0 {
                return Err("capture replay payload limit must be non-zero".to_owned());
            }
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| format!("capture-worker session {session_id} does not exist"))?;
            let metadata = session.index.current_metadata();
            if block >= metadata.total_blocks {
                return Err(format!(
                    "capture block {block} exceeds the {} block capture",
                    metadata.total_blocks
                ));
            }
            let start_channel = usize::try_from(request.start_channel)
                .map_err(|_| "capture replay channel cursor exceeds this host".to_owned())?;
            if start_channel > request.channels.len() {
                return Err("capture replay channel cursor is out of bounds".to_owned());
            }
            let mut payload_bytes = 0_u64;
            let mut blocks = Vec::new();
            let mut next_channel = start_channel;
            while next_channel < request.channels.len() {
                let channel = request.channels[next_channel];
                let channel_index = usize::try_from(channel)
                    .map_err(|_| format!("capture channel {channel} exceeds this host"))?;
                let data = session
                    .index
                    .packed_block(channel_index, block)
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| {
                        "prepared capture index does not support packed replay".to_owned()
                    })?;
                let data_len = u64::try_from(data.len())
                    .map_err(|_| "capture replay block exceeds this host".to_owned())?;
                if !blocks.is_empty()
                    && payload_bytes.saturating_add(data_len) > request.max_payload_bytes
                {
                    break;
                }
                let start_sample = block.saturating_mul(metadata.samples_per_block);
                let valid_samples = metadata
                    .samples_per_block
                    .min(metadata.total_samples.saturating_sub(start_sample));
                blocks.push(CaptureWorkerReplayBlock {
                    channel,
                    block,
                    start_sample,
                    valid_samples,
                    data: data.to_vec(),
                });
                payload_bytes = payload_bytes.saturating_add(data_len);
                next_channel += 1;
            }
            Ok((blocks, next_channel as u64))
        })();
        match result {
            Ok((blocks, next_channel)) => CaptureWorkerMessage::Replay {
                sequence,
                block,
                blocks,
                next_channel,
            },
            Err(message) => CaptureWorkerMessage::Failed { sequence, message },
        }
    }
}

#[cfg(test)]
mod worker_runtime_tests {
    use signal_artifacts::MemoryArtifactRepository;
    use signal_runtime::InlineWorkExecutor;

    use super::*;
    use crate::{
        BlockData, CaptureIndexBuildProgress, CaptureIndexPreparationRequest, CaptureMetadata,
        CaptureSampledChannel, CaptureSampledWindow, CaptureTransition, Result,
    };

    const OPERATION: &str = "test.capture.prepare/v1";

    struct TestFactory {
        identity: SourceIdentity,
        expected_identity: SourceIdentity,
    }

    struct TestIndex {
        identity: SourceIdentity,
        metadata: CaptureMetadata,
    }

    struct TestOpenTask {
        expected_identity: SourceIdentity,
        index: Option<TestIndex>,
        progress_pending: bool,
    }

    impl CaptureIndexOpenTask for TestOpenTask {
        fn expected_index_identity(&self) -> Option<SourceIdentity> {
            Some(self.expected_identity)
        }

        fn step(&mut self) -> Result<CaptureIndexOpenStep> {
            if self.progress_pending {
                self.progress_pending = false;
                return Ok(CaptureIndexOpenStep::Progress(CaptureIndexBuildProgress {
                    completed: 1,
                    total: 1,
                }));
            }
            self.index
                .take()
                .map(|index| CaptureIndexOpenStep::Ready(Box::new(index)))
                .ok_or_else(|| {
                    crate::Error::ParseError("test index task is already complete".to_owned())
                })
        }
    }

    impl CaptureIndexFactory for TestFactory {
        fn display_name(&self) -> String {
            "fixture.dsl".to_owned()
        }

        fn metadata(&self) -> Result<CaptureMetadata> {
            Ok(metadata())
        }

        fn open(
            self: Box<Self>,
            _artifact_repository: Arc<dyn ArtifactRepository>,
            _work_executor: Arc<dyn WorkExecutor>,
            progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
        ) -> Result<Box<dyn CaptureIndex + Send>> {
            progress(CaptureIndexBuildProgress {
                completed: 1,
                total: 1,
            });
            Ok(Box::new(TestIndex {
                identity: self.identity,
                metadata: metadata(),
            }))
        }

        fn open_task(
            self: Box<Self>,
            _artifact_repository: Arc<dyn ArtifactRepository>,
            _work_executor: Arc<dyn WorkExecutor>,
        ) -> Result<Box<dyn CaptureIndexOpenTask>> {
            Ok(Box::new(TestOpenTask {
                expected_identity: self.expected_identity,
                index: Some(TestIndex {
                    identity: self.identity,
                    metadata: metadata(),
                }),
                progress_pending: true,
            }))
        }
    }

    impl CaptureIndex for TestIndex {
        fn display_name(&self) -> String {
            "fixture.dsl".to_owned()
        }

        fn index_identity(&self) -> SourceIdentity {
            self.identity
        }

        fn header(&self) -> &CaptureMetadata {
            &self.metadata
        }

        fn capture_duration_us(&self) -> f64 {
            self.metadata.duration_us()
        }

        fn sampled_window(
            &mut self,
            channels: &[usize],
            start_sample: u64,
            end_sample: u64,
            _target_points: usize,
        ) -> Result<CaptureSampledWindow> {
            Ok(CaptureSampledWindow {
                start_sample,
                end_sample,
                sample_step: 1,
                channels: channels
                    .iter()
                    .map(|channel| CaptureSampledChannel {
                        channel: *channel,
                        name: format!("D{channel}"),
                        initial: false,
                        transitions: vec![CaptureTransition {
                            sample: start_sample + 1,
                            value: true,
                        }],
                        waveform: Vec::new(),
                    })
                    .collect(),
            })
        }

        fn packed_block(&mut self, channel: usize, block: u64) -> Result<Option<BlockData>> {
            Ok(Some(BlockData::from(vec![channel as u8, block as u8, 0])))
        }
    }

    fn metadata() -> CaptureMetadata {
        CaptureMetadata {
            total_probes: 2,
            samplerate: "1 MHz".to_owned(),
            samplerate_hz: 1_000_000.0,
            sample_period: 0.000_001,
            total_samples: 100,
            total_blocks: 1,
            samples_per_block: 100,
            probe_names: vec!["D0".to_owned(), "D1".to_owned()],
            trigger_sample: None,
        }
    }

    fn runtime(identity: SourceIdentity) -> CaptureWorkerRuntime {
        let index_identity = SourceIdentity::from_bytes([42; 32]);
        runtime_with_index_identities(identity, index_identity, index_identity)
    }

    fn runtime_with_index_identities(
        source_identity: SourceIdentity,
        expected_index_identity: SourceIdentity,
        actual_index_identity: SourceIdentity,
    ) -> CaptureWorkerRuntime {
        let mut operations = CaptureWorkerOperationRegistry::new();
        operations
            .register(WorkerOperation::new(OPERATION).unwrap(), move |payload| {
                if payload != b"fixture" {
                    return Err("unknown fixture".to_owned());
                }
                Ok(CaptureWorkerPreparedIndex::new(
                    source_identity,
                    Box::new(TestFactory {
                        identity: actual_index_identity,
                        expected_identity: expected_index_identity,
                    }),
                ))
            })
            .unwrap();
        CaptureWorkerRuntime::new(
            operations,
            Arc::new(MemoryArtifactRepository::new()),
            Arc::new(InlineWorkExecutor),
        )
    }

    #[test]
    fn preparation_rejects_an_index_that_disagrees_with_its_builder() {
        let mut runtime = runtime_with_index_identities(
            SourceIdentity::from_bytes([7; 32]),
            SourceIdentity::from_bytes([42; 32]),
            SourceIdentity::from_bytes([43; 32]),
        );

        let messages = runtime.execute(CaptureWorkerRequest::Prepare {
            sequence: 1,
            request: CaptureIndexPreparationRequest::new(
                WorkerOperation::new(OPERATION).unwrap(),
                b"fixture".to_vec(),
            ),
        });

        assert!(matches!(
            messages.as_slice(),
            [
                CaptureWorkerMessage::Metadata { sequence: 1, .. },
                CaptureWorkerMessage::Progress { sequence: 1, .. },
                CaptureWorkerMessage::Failed { sequence: 1, message },
            ] if message.contains("identity declared by its builder")
        ));
    }

    #[test]
    fn preparation_query_and_release_share_one_owned_session() {
        let identity = SourceIdentity::from_bytes([7; 32]);
        let mut runtime = runtime(identity);
        let prepared = runtime.execute(CaptureWorkerRequest::Prepare {
            sequence: 1,
            request: CaptureIndexPreparationRequest::new(
                WorkerOperation::new(OPERATION).unwrap(),
                b"fixture".to_vec(),
            ),
        });

        assert!(matches!(
            prepared.as_slice(),
            [
                CaptureWorkerMessage::Metadata { sequence: 1, .. },
                CaptureWorkerMessage::Progress { sequence: 1, .. },
                CaptureWorkerMessage::Prepared {
                    sequence: 1,
                    session_id: 1,
                    source_identity: actual_source_identity,
                    index_identity: actual_index_identity,
                    ..
                }
            ] if *actual_source_identity == identity
                && *actual_index_identity == SourceIdentity::from_bytes([42; 32])
        ));

        let queried = runtime.execute(CaptureWorkerRequest::Query {
            sequence: 2,
            session_id: 1,
            query: super::super::query::CaptureIndexQuery {
                channels: vec![1],
                start_sample: 10,
                end_sample: 20,
                target_points: 100,
            },
        });
        assert!(matches!(
            queried.as_slice(),
            [CaptureWorkerMessage::Window { sequence: 2, window }]
                if window.channels[0].channel == 1
        ));

        let replayed = runtime.execute(CaptureWorkerRequest::Replay {
            sequence: 4,
            session_id: 1,
            request: CaptureWorkerReplayRequest {
                channels: vec![0, 1],
                block: 0,
                start_channel: 0,
                max_payload_bytes: 3,
            },
        });
        assert!(matches!(
            replayed.as_slice(),
            [CaptureWorkerMessage::Replay {
                sequence: 4,
                block: 0,
                blocks,
                next_channel: 1,
            }] if blocks[0].channel == 0 && blocks[0].data == [0, 0, 0]
        ));

        assert!(
            runtime
                .execute(CaptureWorkerRequest::Release { session_id: 1 })
                .is_empty()
        );
        assert!(matches!(
            runtime
                .execute(CaptureWorkerRequest::Query {
                    sequence: 3,
                    session_id: 1,
                    query: super::super::query::CaptureIndexQuery {
                        channels: vec![0],
                        start_sample: 0,
                        end_sample: 10,
                        target_points: 10,
                    },
                })
                .as_slice(),
            [CaptureWorkerMessage::Failed { sequence: 3, message }]
                if message.contains("does not exist")
        ));
    }

    #[test]
    fn unknown_operations_fail_without_creating_a_session() {
        let identity = SourceIdentity::from_bytes([7; 32]);
        let mut runtime = runtime(identity);

        let messages = runtime.execute(CaptureWorkerRequest::Prepare {
            sequence: 4,
            request: CaptureIndexPreparationRequest::new(
                WorkerOperation::new("missing.capture/v1").unwrap(),
                Vec::new(),
            ),
        });

        assert!(matches!(
            messages.as_slice(),
            [CaptureWorkerMessage::Failed { sequence: 4, message }]
                if message.contains("not registered")
        ));
    }

    #[test]
    fn preparation_can_be_cancelled_between_bounded_steps() {
        let identity = SourceIdentity::from_bytes([8; 32]);
        let mut runtime = runtime(identity);
        let mut messages = Vec::new();
        runtime.execute_streaming(
            CaptureWorkerRequest::Prepare {
                sequence: 8,
                request: CaptureIndexPreparationRequest::new(
                    WorkerOperation::new(OPERATION).unwrap(),
                    b"fixture".to_vec(),
                ),
            },
            &mut |message| messages.push(message),
        );

        assert!(runtime.has_pending_preparations());
        assert!(matches!(
            messages.as_slice(),
            [CaptureWorkerMessage::Metadata { sequence: 8, .. }]
        ));

        messages.clear();
        assert!(runtime.advance_streaming(&mut |message| messages.push(message)));
        assert!(matches!(
            messages.as_slice(),
            [CaptureWorkerMessage::Progress { sequence: 8, .. }]
        ));

        messages.clear();
        runtime.execute_streaming(
            CaptureWorkerRequest::Cancel { sequence: 8 },
            &mut |message| messages.push(message),
        );
        assert_eq!(messages, [CaptureWorkerMessage::Cancelled { sequence: 8 }]);
        assert!(!runtime.has_pending_preparations());
        assert!(!runtime.advance_streaming(&mut |_| {}));
    }

    #[test]
    fn repeated_identity_preparation_leases_one_session() {
        let identity = SourceIdentity::from_bytes([9; 32]);
        let mut runtime = runtime(identity);
        let prepare = |sequence| CaptureWorkerRequest::Prepare {
            sequence,
            request: CaptureIndexPreparationRequest::new(
                WorkerOperation::new(OPERATION).unwrap(),
                b"fixture".to_vec(),
            ),
        };

        let first = runtime.execute(prepare(10));
        let second = runtime.execute(prepare(11));

        assert!(matches!(
            first.last(),
            Some(CaptureWorkerMessage::Prepared { session_id: 1, .. })
        ));
        assert!(matches!(
            second.as_slice(),
            [
                CaptureWorkerMessage::Metadata { sequence: 11, .. },
                CaptureWorkerMessage::Prepared {
                    sequence: 11,
                    session_id: 1,
                    ..
                }
            ]
        ));

        runtime.execute(CaptureWorkerRequest::Release { session_id: 1 });
        assert!(matches!(
            runtime
                .execute(CaptureWorkerRequest::Query {
                    sequence: 12,
                    session_id: 1,
                    query: super::super::query::CaptureIndexQuery {
                        channels: vec![0],
                        start_sample: 0,
                        end_sample: 10,
                        target_points: 10,
                    },
                })
                .as_slice(),
            [CaptureWorkerMessage::Window { sequence: 12, .. }]
        ));
        runtime.execute(CaptureWorkerRequest::Release { session_id: 1 });
        assert!(matches!(
            runtime
                .execute(CaptureWorkerRequest::Query {
                    sequence: 13,
                    session_id: 1,
                    query: super::super::query::CaptureIndexQuery {
                        channels: vec![0],
                        start_sample: 0,
                        end_sample: 10,
                        target_points: 10,
                    },
                })
                .as_slice(),
            [CaptureWorkerMessage::Failed { sequence: 13, .. }]
        ));
    }
}
