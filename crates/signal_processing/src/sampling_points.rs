use std::sync::{Arc, Mutex, RwLock};

use crossbeam_channel::{Receiver, Sender};

use crate::derived_word_store::{
    AnnotationQuery, IndexedAnnotationStore, IndexedAnnotationWriter, LiveStoreConfig,
    PersistentStoreConfig, StoreError, StoreResult,
};
use crate::{CollectedWordLaneQuery, DerivedLanes, Word, WordPayload, WorkExecutor, WorkTask};

const INLINE_VALUE_BITS: usize = 57;
const INLINE_VALUE_SHIFT: usize = 7;
const PERSISTENT_BATCH_QUEUE_DEPTH: usize = 8;

/// One sampling decision produced by a clocked processing node.
///
/// Values follow the order of the node's declared sampled inputs. The
/// record deliberately carries no protocol or viewer knowledge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SamplingPoint {
    /// Shared timeline timestamp at which the node accepted input values.
    pub time_ns: u64,
    /// Clock level associated with the accepted sample.
    pub clock_high: bool,
    /// Input logic values in the node's declared sampled-input order.
    pub values: Vec<bool>,
}

impl SamplingPoint {
    /// Creates one sampling decision in the shared nanosecond time domain.
    ///
    /// # Parameters
    /// - `time_ns`: Timestamp at which the node accepted the values.
    /// - `clock_high`: Clock level associated with the decision.
    /// - `values`: Input values in declared sampled-input order.
    pub fn new(time_ns: u64, clock_high: bool, values: impl Into<Vec<bool>>) -> Self {
        Self {
            time_ns,
            clock_high,
            values: values.into(),
        }
    }
}

/// Allocation-free sampling decision used while a decoder is processing.
///
/// The packed representation is intentionally protocol-neutral. Values keep
/// the same least-significant-bit-first order as [`SamplingPoint::values`]
/// and are expanded only when a presentation query requests them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedSamplingPoint {
    time_ns: u64,
    values: u64,
    value_count: u8,
    clock_high: bool,
}

/// Opaque storage-ready batch of packed sampling decisions.
///
/// Producers fill this batch directly while assembling their output. The
/// sampling store retains ownership of the persistent word representation,
/// avoiding a second intermediate point vector and conversion pass.
pub struct PackedSamplingPointBatch {
    words: Vec<Word>,
}

impl PackedSamplingPointBatch {
    /// Allocates a batch for at most the expected number of decisions.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            words: Vec::with_capacity(capacity),
        }
    }

    /// Appends one packed decision in the store's canonical representation.
    pub fn push(&mut self, point: PackedSamplingPoint) {
        self.words.push(encode_packed_point(point));
    }

    /// Returns whether the batch contains no decisions.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

impl PackedSamplingPoint {
    /// Packs up to 64 sampled logic values for allocation-free processing.
    pub fn new(time_ns: u64, clock_high: bool, values: u64, value_count: usize) -> Self {
        assert!(value_count <= u64::BITS as usize);
        let values = match value_count {
            0 => 0,
            64 => values,
            count => values & ((1_u64 << count) - 1),
        };
        Self {
            time_ns,
            values,
            value_count: value_count as u8,
            clock_high,
        }
    }

    fn unpack(self) -> SamplingPoint {
        SamplingPoint::new(
            self.time_ns,
            self.clock_high,
            (0..usize::from(self.value_count))
                .map(|index| self.values & (1_u64 << index) != 0)
                .collect::<Vec<_>>(),
        )
    }
}

/// Random-access source of sampling decisions for a processed time range.
///
/// Concrete processing nodes implement their own sampling semantics. The
/// generic store and its presentation consumers only request already-accepted
/// decisions for the visible range.
pub trait SamplingPointProvider: std::fmt::Debug + Send + Sync {
    /// Returns every accepted point in the range, or `None` when the complete
    /// range is denser than `minimum_spacing_ns` and should remain hidden.
    fn points_in_range_with_minimum_spacing(
        &self,
        start_ns: u64,
        end_ns: u64,
        minimum_spacing_ns: u64,
    ) -> Option<Vec<SamplingPoint>>;
}

struct RetainedWordSamplingProvider {
    lanes: DerivedLanes,
    lane_name: String,
    clock_high: bool,
    value_count: usize,
}

impl std::fmt::Debug for RetainedWordSamplingProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetainedWordSamplingProvider")
            .field("lane_name", &self.lane_name)
            .field("clock_high", &self.clock_high)
            .field("value_count", &self.value_count)
            .finish()
    }
}

impl SamplingPointProvider for RetainedWordSamplingProvider {
    fn points_in_range_with_minimum_spacing(
        &self,
        start_ns: u64,
        end_ns: u64,
        minimum_spacing_ns: u64,
    ) -> Option<Vec<SamplingPoint>> {
        if start_ns > end_ns {
            return Some(Vec::new());
        }
        let lane = self
            .lanes
            .opaque_lanes()
            .into_iter()
            .find(|lane| lane.name() == self.lane_name)?;
        let indexed = lane.query::<CollectedWordLaneQuery>()?.indexed_lane()?;
        let maximum_visible = match end_ns
            .saturating_sub(start_ns)
            .checked_div(minimum_spacing_ns)
        {
            Some(intervals) => usize::try_from(intervals).ok()?.saturating_add(1),
            None => usize::try_from(indexed.metadata().total_word_count).ok()?,
        };
        let window = indexed
            .query()
            .exact_window(start_ns, end_ns, maximum_visible.saturating_add(4).max(1))
            .ok()?;
        if !window.complete {
            return None;
        }
        let points = window
            .annotations
            .into_iter()
            .filter(|annotation| (start_ns..=end_ns).contains(&annotation.start_ns))
            .map(|annotation| {
                SamplingPoint::new(
                    annotation.start_ns,
                    self.clock_high,
                    (0..self.value_count)
                        .map(|bit| annotation.value & (1_u64 << bit) != 0)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        if points.len() > maximum_visible
            || points
                .windows(2)
                .any(|pair| pair[1].time_ns.saturating_sub(pair[0].time_ns) < minimum_spacing_ns)
        {
            None
        } else {
            Some(points)
        }
    }
}

/// Thread-safe, visible-range store of sampling decisions produced by a node.
///
/// Transient stores retain chronological batches in memory and may delegate
/// queries to a node-owned provider. Compiler-configured persistent stores use
/// the shared indexed artifact repository so stable captures can reopen the
/// same decisions without executing their decoder again.
struct SamplingPointStoreInner {
    points: RwLock<Vec<SamplingPoint>>,
    provider: RwLock<Option<Arc<dyn SamplingPointProvider>>>,
    persistent: Option<PersistentSamplingPoints>,
}

struct PersistentSamplingPoints {
    store: IndexedAnnotationStore,
    writer: PersistentSamplingWriter,
}

enum PersistentSamplingWriter {
    ReadOnly,
    Direct(Box<Mutex<Option<IndexedAnnotationWriter>>>),
    Queued(QueuedSamplingWriter),
}

struct QueuedSamplingWriter {
    sender: Mutex<Option<Sender<Vec<Word>>>>,
    task: Mutex<Option<Box<dyn WorkTask>>>,
    result: Arc<Mutex<Option<Result<(), String>>>>,
}

#[derive(Clone)]
pub struct SamplingPointStore {
    inner: Arc<SamplingPointStoreInner>,
}

impl std::fmt::Debug for SamplingPointStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SamplingPointStore")
            .field("has_provider", &self.has_provider())
            .field("is_persistent", &self.is_persistent())
            .finish()
    }
}

impl Default for SamplingPointStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(SamplingPointStoreInner {
                points: RwLock::new(Vec::new()),
                provider: RwLock::new(None),
                persistent: None,
            }),
        }
    }
}

impl SamplingPointStore {
    /// Uses an already-retained indexed word lane whose events are identical
    /// to this node's sampling decisions.
    ///
    /// # Parameters
    /// - `lanes`: Derived-lane catalog containing the retained word source.
    /// - `lane_name`: Stable retained-word lane name.
    /// - `clock_high`: Clock level associated with retained events.
    /// - `value_count`: Number of sampled values packed into each word.
    pub fn set_retained_word_provider(
        &self,
        lanes: DerivedLanes,
        lane_name: impl Into<String>,
        clock_high: bool,
        value_count: usize,
    ) {
        self.set_provider(Arc::new(RetainedWordSamplingProvider {
            lanes,
            lane_name: lane_name.into(),
            clock_high,
            value_count,
        }));
    }

    /// Creates a store backed by the shared persistent annotation repository.
    pub fn create_persistent(
        config: PersistentStoreConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> StoreResult<Self> {
        let live_config = LiveStoreConfig {
            persistence: Some(config),
            work_executor: Arc::clone(&work_executor),
            ..LiveStoreConfig::default()
        };
        let (writer, store) = IndexedAnnotationWriter::create(live_config)?;
        let writer = if work_executor.supports_long_running_tasks() {
            PersistentSamplingWriter::Queued(start_queued_writer(writer, work_executor)?)
        } else {
            PersistentSamplingWriter::Direct(Box::new(Mutex::new(Some(writer))))
        };
        Ok(Self {
            inner: Arc::new(SamplingPointStoreInner {
                points: RwLock::new(Vec::new()),
                provider: RwLock::new(None),
                persistent: Some(PersistentSamplingPoints { store, writer }),
            }),
        })
    }

    /// Opens an existing persistent store as a read-only sampling-point source.
    pub fn open_persistent(config: &PersistentStoreConfig) -> StoreResult<Option<Self>> {
        let Some(store) = IndexedAnnotationStore::open_persistent(config)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            inner: Arc::new(SamplingPointStoreInner {
                points: RwLock::new(Vec::new()),
                provider: RwLock::new(None),
                persistent: Some(PersistentSamplingPoints {
                    store,
                    writer: PersistentSamplingWriter::ReadOnly,
                }),
            }),
        }))
    }

    /// Installs a node-owned visible-range provider for transient queries.
    pub fn set_provider(&self, provider: Arc<dyn SamplingPointProvider>) {
        if self.inner.persistent.is_some() {
            return;
        }
        *self.inner.provider.write().unwrap() = Some(provider);
    }

    /// Returns whether the store delegates visible-range queries to a provider.
    pub fn has_provider(&self) -> bool {
        self.inner.provider.read().unwrap().is_some()
    }

    /// Returns whether the store is backed by persistent indexed annotations.
    pub fn is_persistent(&self) -> bool {
        self.inner.persistent.is_some()
    }

    /// Appends or replaces one sampling decision by timestamp.
    ///
    /// # Parameters
    /// - `point`: Sampling decision to retain or persist.
    pub fn record(&self, point: SamplingPoint) -> StoreResult<()> {
        self.record_batch([point])
    }

    /// Appends a chronological batch, replacing values at duplicate timestamps.
    pub fn record_batch(&self, points: impl IntoIterator<Item = SamplingPoint>) -> StoreResult<()> {
        if self.inner.provider.read().unwrap().is_some() {
            return Ok(());
        }
        if let Some(persistent) = &self.inner.persistent {
            let words = points.into_iter().map(encode_point).collect::<Vec<_>>();
            if words.is_empty() {
                return Ok(());
            }
            return persistent.writer.append(words);
        }
        let mut points = points.into_iter().peekable();
        let Some(first) = points.peek() else {
            return Ok(());
        };

        let mut stored = self.inner.points.write().unwrap();
        let keep = stored.partition_point(|point| point.time_ns < first.time_ns);
        if keep < stored.len() {
            stored.truncate(keep);
        }

        for point in points {
            if stored
                .last()
                .is_some_and(|previous| previous.time_ns > point.time_ns)
            {
                let keep = stored.partition_point(|stored| stored.time_ns < point.time_ns);
                stored.truncate(keep);
            }
            if stored
                .last()
                .is_some_and(|previous| previous.time_ns == point.time_ns)
            {
                stored.pop();
            }
            stored.push(point);
        }
        Ok(())
    }

    /// Appends packed sampling decisions without allocating intermediate vectors.
    pub fn record_packed_batch(
        &self,
        points: impl IntoIterator<Item = PackedSamplingPoint>,
    ) -> StoreResult<()> {
        if self.inner.provider.read().unwrap().is_some() {
            return Ok(());
        }
        if let Some(persistent) = &self.inner.persistent {
            let words = points
                .into_iter()
                .map(encode_packed_point)
                .collect::<Vec<_>>();
            if words.is_empty() {
                return Ok(());
            }
            return persistent.writer.append(words);
        }
        self.record_batch(points.into_iter().map(PackedSamplingPoint::unpack))
    }

    /// Appends a storage-ready packed batch to a persistent sampling store.
    ///
    /// This path preserves the same encoding and queued-writer behavior as
    /// [`Self::record_packed_batch`] while allowing a producer to avoid an
    /// intermediate `Vec<PackedSamplingPoint>`.
    pub fn record_packed_word_batch(&self, batch: PackedSamplingPointBatch) -> StoreResult<()> {
        if self.inner.provider.read().unwrap().is_some() || batch.is_empty() {
            return Ok(());
        }
        let Some(persistent) = &self.inner.persistent else {
            return Err(StoreError::Persistent(
                "storage-ready sampling batches require a persistent store".into(),
            ));
        };
        persistent.writer.append(batch.words)
    }

    /// Finalizes queued persistent writes so all points become queryable.
    pub fn finish(&self) -> StoreResult<()> {
        let Some(persistent) = &self.inner.persistent else {
            return Ok(());
        };
        persistent.writer.finish()
    }

    /// Returns all retained decisions in the inclusive time range.
    pub fn points_in_range(&self, start_ns: u64, end_ns: u64) -> Vec<SamplingPoint> {
        self.points_in_range_with_minimum_spacing(start_ns, end_ns, 0)
            .unwrap_or_default()
    }

    /// Returns the complete range only when every adjacent point meets the
    /// requested spacing. This lets presentation consumers enforce an
    /// all-or-nothing density policy without first cloning a dense range.
    ///
    /// # Parameters
    /// - `start_ns`: Inclusive start of the shared timeline range.
    /// - `end_ns`: Inclusive end of the shared timeline range.
    /// - `minimum_spacing_ns`: Minimum separation required between returned points.
    pub fn points_in_range_with_minimum_spacing(
        &self,
        start_ns: u64,
        end_ns: u64,
        minimum_spacing_ns: u64,
    ) -> Option<Vec<SamplingPoint>> {
        if start_ns > end_ns {
            return Some(Vec::new());
        }
        let provider = self.inner.provider.read().unwrap().clone();
        if let Some(provider) = provider {
            return provider.points_in_range_with_minimum_spacing(
                start_ns,
                end_ns,
                minimum_spacing_ns,
            );
        }
        if let Some(persistent) = &self.inner.persistent {
            return persistent_points_in_range(
                &persistent.store,
                start_ns,
                end_ns,
                minimum_spacing_ns,
            );
        }
        let stored = self.inner.points.read().unwrap();
        let start = stored.partition_point(|point| point.time_ns < start_ns);
        let end = stored.partition_point(|point| point.time_ns <= end_ns);
        let visible = &stored[start..end];
        if visible
            .windows(2)
            .any(|pair| pair[1].time_ns.saturating_sub(pair[0].time_ns) < minimum_spacing_ns)
        {
            return None;
        }
        Some(visible.to_vec())
    }

    /// Removes transient in-memory decisions without changing a persistent store.
    pub fn clear(&self) {
        self.inner.points.write().unwrap().clear();
    }

    /// Returns whether empty.
    pub fn is_empty(&self) -> bool {
        if let Some(persistent) = &self.inner.persistent {
            return persistent.store.metadata().total_word_count == 0;
        }
        self.inner.points.read().unwrap().is_empty()
    }
}

impl PersistentSamplingWriter {
    fn append(&self, words: Vec<Word>) -> StoreResult<()> {
        match self {
            Self::ReadOnly => Ok(()),
            Self::Direct(writer) => {
                let mut writer = writer.lock().unwrap();
                let Some(writer) = writer.as_mut() else {
                    return Ok(());
                };
                writer.append_batch(&words)
            }
            Self::Queued(writer) => writer.append(words),
        }
    }

    fn finish(&self) -> StoreResult<()> {
        match self {
            Self::ReadOnly => Ok(()),
            Self::Direct(writer) => {
                let Some(mut writer) = writer.lock().unwrap().take() else {
                    return Ok(());
                };
                writer.finish()
            }
            Self::Queued(writer) => writer.finish(),
        }
    }
}

impl QueuedSamplingWriter {
    fn append(&self, words: Vec<Word>) -> StoreResult<()> {
        let sender = self.sender.lock().unwrap();
        let Some(sender) = sender.as_ref() else {
            return self.completed_result();
        };
        sender.send(words).map_err(|_| {
            StoreError::Persistent(
                self.failure_message()
                    .unwrap_or_else(|| "sampling-point cache writer stopped".into()),
            )
        })
    }

    fn finish(&self) -> StoreResult<()> {
        self.sender.lock().unwrap().take();
        if let Some(task) = self.task.lock().unwrap().take() {
            task.wait();
        }
        self.completed_result()
    }

    fn completed_result(&self) -> StoreResult<()> {
        match self.result.lock().unwrap().as_ref() {
            Some(Ok(())) => Ok(()),
            Some(Err(message)) => Err(StoreError::Persistent(message.clone())),
            None => Err(StoreError::Persistent(
                "sampling-point cache writer has not completed".into(),
            )),
        }
    }

    fn failure_message(&self) -> Option<String> {
        self.result
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|result| result.as_ref().err().cloned())
    }
}

fn start_queued_writer(
    writer: IndexedAnnotationWriter,
    work_executor: Arc<dyn WorkExecutor>,
) -> StoreResult<QueuedSamplingWriter> {
    let (sender, receiver) = crossbeam_channel::bounded(PERSISTENT_BATCH_QUEUE_DEPTH);
    let result = Arc::new(Mutex::new(None));
    let worker_result = Arc::clone(&result);
    let task = work_executor
        .submit_long_running(Box::new(move || {
            let outcome = run_queued_writer(writer, receiver).map_err(|error| error.to_string());
            *worker_result.lock().unwrap() = Some(outcome);
        }))
        .map_err(StoreError::Persistent)?;
    Ok(QueuedSamplingWriter {
        sender: Mutex::new(Some(sender)),
        task: Mutex::new(Some(task)),
        result,
    })
}

fn run_queued_writer(
    mut writer: IndexedAnnotationWriter,
    receiver: Receiver<Vec<Word>>,
) -> StoreResult<()> {
    for words in receiver {
        writer.append_batch(&words)?;
    }
    writer.finish()
}

fn encode_point(point: SamplingPoint) -> Word {
    let clock = u64::from(point.clock_high);
    if point.values.len() <= INLINE_VALUE_BITS {
        let mut value = clock | ((point.values.len() as u64) << 1);
        for (index, set) in point.values.into_iter().enumerate() {
            if set {
                value |= 1_u64 << (INLINE_VALUE_SHIFT + index);
            }
        }
        Word::new(value, point.time_ns)
    } else {
        let value_count = point.values.len();
        let mut packed = vec![0_u8; value_count.div_ceil(u8::BITS as usize)];
        for (index, set) in point.values.into_iter().enumerate() {
            if set {
                packed[index / u8::BITS as usize] |= 1 << (index % u8::BITS as usize);
            }
        }
        Word::bytes_with_tag(
            clock | ((value_count as u64) << 1),
            packed,
            point.time_ns,
            0,
        )
    }
}

fn encode_packed_point(point: PackedSamplingPoint) -> Word {
    let clock = u64::from(point.clock_high);
    let value_count = usize::from(point.value_count);
    if value_count <= INLINE_VALUE_BITS {
        Word::new(
            clock | ((value_count as u64) << 1) | (point.values << INLINE_VALUE_SHIFT),
            point.time_ns,
        )
    } else {
        let packed = point.values.to_le_bytes();
        Word::bytes_with_tag(
            clock | ((value_count as u64) << 1),
            Arc::<[u8]>::from(&packed[..value_count.div_ceil(u8::BITS as usize)]),
            point.time_ns,
            0,
        )
    }
}

fn decode_point(annotation: crate::Annotation) -> Option<SamplingPoint> {
    let clock_high = annotation.value & 1 != 0;
    let values: Vec<bool> = match annotation.payload {
        None => {
            let count = usize::try_from((annotation.value >> 1) & 0x3f).ok()?;
            (0..count)
                .map(|index| annotation.value & (1_u64 << (INLINE_VALUE_SHIFT + index)) != 0)
                .collect()
        }
        Some(WordPayload::Bytes(bytes)) => {
            let count = usize::try_from(annotation.value >> 1).ok()?;
            if count > bytes.len().saturating_mul(u8::BITS as usize) {
                return None;
            }
            (0..count)
                .map(|index| bytes[index / u8::BITS as usize] & (1 << (index % 8)) != 0)
                .collect()
        }
        Some(WordPayload::Text(_)) => return None,
    };
    Some(SamplingPoint::new(annotation.start_ns, clock_high, values))
}

fn persistent_points_in_range(
    store: &IndexedAnnotationStore,
    start_ns: u64,
    end_ns: u64,
    minimum_spacing_ns: u64,
) -> Option<Vec<SamplingPoint>> {
    let maximum_visible = match end_ns
        .saturating_sub(start_ns)
        .checked_div(minimum_spacing_ns)
    {
        Some(intervals) => usize::try_from(intervals).ok()?.saturating_add(1),
        None => usize::try_from(store.metadata().total_word_count).ok()?,
    };
    let window = store
        .exact_window(start_ns, end_ns, maximum_visible.saturating_add(4).max(1))
        .ok()?;
    if !window.complete {
        return None;
    }
    let points = window
        .annotations
        .into_iter()
        .filter(|annotation| (start_ns..=end_ns).contains(&annotation.start_ns))
        .map(decode_point)
        .collect::<Option<Vec<_>>>()?;
    if points.len() > maximum_visible
        || points
            .windows(2)
            .any(|pair| pair[1].time_ns.saturating_sub(pair[0].time_ns) < minimum_spacing_ns)
    {
        None
    } else {
        Some(points)
    }
}

#[cfg(test)]
mod sampling_point_store_tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        ArtifactRepository, InlineWorkExecutor, MemoryArtifactRepository, WorkExecutorTask,
    };

    struct ThreadWorkExecutor;

    impl WorkExecutor for ThreadWorkExecutor {
        fn available_parallelism(&self) -> usize {
            4
        }

        fn supports_long_running_tasks(&self) -> bool {
            true
        }

        fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
            let handle = std::thread::spawn(task);
            Ok(Box::new(ThreadWorkTask(Some(handle))))
        }
    }

    struct ThreadWorkTask(Option<std::thread::JoinHandle<()>>);

    impl WorkTask for ThreadWorkTask {
        fn is_finished(&self) -> bool {
            self.0
                .as_ref()
                .is_none_or(std::thread::JoinHandle::is_finished)
        }

        fn wait(mut self: Box<Self>) {
            if let Some(handle) = self.0.take() {
                handle.join().unwrap();
            }
        }
    }

    #[derive(Debug)]
    struct FixedProvider;

    impl SamplingPointProvider for FixedProvider {
        fn points_in_range_with_minimum_spacing(
            &self,
            start_ns: u64,
            end_ns: u64,
            minimum_spacing_ns: u64,
        ) -> Option<Vec<SamplingPoint>> {
            let points = [
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(20, false, vec![true]),
            ]
            .into_iter()
            .filter(|point| (start_ns..=end_ns).contains(&point.time_ns))
            .collect::<Vec<_>>();
            if points
                .windows(2)
                .any(|pair| pair[1].time_ns.saturating_sub(pair[0].time_ns) < minimum_spacing_ns)
            {
                None
            } else {
                Some(points)
            }
        }
    }

    #[test]
    fn visible_range_is_inclusive_and_ordered() {
        let store = SamplingPointStore::default();
        store
            .record_batch([
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(20, false, vec![true]),
                SamplingPoint::new(30, true, vec![false]),
            ])
            .unwrap();

        assert_eq!(
            store.points_in_range(20, 30),
            vec![
                SamplingPoint::new(20, false, vec![true]),
                SamplingPoint::new(30, true, vec![false]),
            ]
        );
    }

    #[test]
    fn recording_from_an_earlier_time_replaces_stale_points() {
        let store = SamplingPointStore::default();
        store
            .record_batch([
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(30, true, vec![false]),
            ])
            .unwrap();
        store
            .record_batch([
                SamplingPoint::new(20, false, vec![true]),
                SamplingPoint::new(40, false, vec![true]),
            ])
            .unwrap();

        assert_eq!(
            store.points_in_range(0, u64::MAX),
            vec![
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(20, false, vec![true]),
                SamplingPoint::new(40, false, vec![true]),
            ]
        );
    }

    #[test]
    fn minimum_spacing_rejects_the_complete_dense_range() {
        let store = SamplingPointStore::default();
        store
            .record_batch([
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(14, false, vec![true]),
                SamplingPoint::new(30, true, vec![false]),
            ])
            .unwrap();

        assert!(
            store
                .points_in_range_with_minimum_spacing(0, 40, 5)
                .is_none()
        );
        assert_eq!(
            store
                .points_in_range_with_minimum_spacing(0, 40, 4)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn provider_serves_ranges_instead_of_recorded_points() {
        let store = SamplingPointStore::default();
        store.set_provider(Arc::new(FixedProvider));
        store
            .record(SamplingPoint::new(15, true, vec![true]))
            .unwrap();

        assert_eq!(
            store.points_in_range(10, 20),
            vec![
                SamplingPoint::new(10, true, vec![false]),
                SamplingPoint::new(20, false, vec![true]),
            ]
        );
        assert!(
            store
                .points_in_range_with_minimum_spacing(10, 20, 11)
                .is_none()
        );
    }

    #[test]
    fn persistent_store_reopens_inline_and_arbitrary_width_points() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let config = PersistentStoreConfig::new([0x53; 32])
            .with_artifact_repository(Arc::clone(&repository));
        let store =
            SamplingPointStore::create_persistent(config.clone(), Arc::new(InlineWorkExecutor))
                .unwrap();
        let wide_values = (0..73).map(|index| index % 3 == 0).collect::<Vec<_>>();
        let expected = vec![
            SamplingPoint::new(10, true, vec![false, true]),
            SamplingPoint::new(20, false, wide_values),
            SamplingPoint::new(40, true, vec![true, false]),
        ];
        store.record_batch(expected.clone()).unwrap();
        store.finish().unwrap();
        drop(store);

        let reopened = SamplingPointStore::open_persistent(&config)
            .unwrap()
            .expect("published sampling points should reopen");

        assert_eq!(reopened.points_in_range(0, 50), expected);
        assert!(
            reopened
                .points_in_range_with_minimum_spacing(0, 50, 15)
                .is_none()
        );
        assert_eq!(
            reopened
                .points_in_range_with_minimum_spacing(0, 50, 10)
                .unwrap(),
            expected
        );
    }

    #[test]
    fn queued_packed_store_reopens_without_decoder_owned_allocations() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let config = PersistentStoreConfig::new([0x71; 32])
            .with_artifact_repository(Arc::clone(&repository));
        let store =
            SamplingPointStore::create_persistent(config.clone(), Arc::new(ThreadWorkExecutor))
                .unwrap();
        let mut batch = PackedSamplingPointBatch::with_capacity(2);
        batch.push(PackedSamplingPoint::new(10, true, 0b101, 3));
        batch.push(PackedSamplingPoint::new(
            20,
            false,
            0xfeed_beef_dead_cafe,
            64,
        ));
        store.record_packed_word_batch(batch).unwrap();
        store.finish().unwrap();
        drop(store);

        let reopened = SamplingPointStore::open_persistent(&config)
            .unwrap()
            .expect("queued sampling points should be published");
        assert_eq!(
            reopened.points_in_range(0, 30),
            vec![
                SamplingPoint::new(10, true, vec![true, false, true]),
                SamplingPoint::new(
                    20,
                    false,
                    (0..64)
                        .map(|bit| 0xfeed_beef_dead_cafe_u64 & (1 << bit) != 0)
                        .collect::<Vec<_>>(),
                ),
            ]
        );
    }
}
