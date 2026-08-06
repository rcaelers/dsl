#[path = "../tests/integration_tests_support/mod.rs"]
mod integration_tests_support;

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::{Color32, Event, Id, Pos2, Rect, UiBuilder};

use logic_analyzer_graph_compiler::GraphLowerer;
use logic_analyzer_graph_plan::{OutputSubscriptionPlan, ProcessingGraph, ProcessingGraphError};
use logic_analyzer_graph_runtime::{
    ApplyError, ApplySummary, GraphRunContext, GraphRuntime, InlineSourcePreparationExecutor,
    LiveRun, SourceProcessOverrides,
};
use logic_analyzer_processing::nodes::decoders::parallel_decoder::{
    ParallelDecoder, ParallelInputStrategy, StrobeMode,
};
use logic_analyzer_processing::nodes::decoders::spi_decoder::{SpiDecoder, SpiMode};
use logic_analyzer_processing::nodes::logic::logic_gate::{GateOp, LogicGate};
use logic_analyzer_processing::nodes::logic::sr_latch::SrLatch;
use logic_analyzer_processing::nodes::logic::text_formatter::TextFormatter;
use logic_analyzer_processing::nodes::logic::trigger_counter::TriggerCounter;
use logic_analyzer_processing::nodes::logic::word_matcher::WordMatcher;
use logic_analyzer_processing::nodes::sinks::binary_file_writer::BinaryFileWriter;
use logic_analyzer_processing::nodes::sinks::{OutputFile, OutputStorage};
use logic_analyzer_processing::nodes::sources::dsl_file::{
    DslFileSource, DslFileSourceConfig, DslFileSourceFactory,
};
use logic_analyzer_processing::types::CsPolarity;
use logic_analyzer_processing::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, ProcessNodeConstruction,
};
use logic_analyzer_viewer::{
    LogicAnalyzerViewer, ViewerLaneBadge, WaveformPresentationRegistry, viewer_lane_renderer,
};
use node_graph::{NodeGraphWidget, NodeId, SocketDirection, SocketId};
use signal_artifacts::{
    ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactRepository, MemoryArtifactRepository,
    ReadArtifact, RepositoryCapabilities, RepositoryError, SourceIdentity, WriteArtifact,
};
use signal_derived::{
    CollectedLaneQuery, CollectedLaneSnapshotRequest, CollectedWordLaneQuery, DerivedLanes,
    OpaqueCollectedLane, OpaqueCollectedLaneSnapshot, TriggerLaneSnapshot,
};
use signal_runtime::{
    AppManager, AppManagerBackend, AppManagerFactory, ConfigurationBoundary, DisconnectEvent,
    InputSub, NodeConfig, NodeSpec, Pipeline, PipelineManager, ProcessNode, WorkExecutor,
    WorkExecutorTask, WorkTask,
};

use integration_tests_support as nodes;

const STARTUP_OUTPUTS: [(&str, &str); 7] = [
    ("SPI Decoder", "MOSI Bits"),
    ("SPI Decoder", "MOSI Data"),
    ("Match Start", "Match"),
    ("Match Stop", "Match"),
    ("SR Flip-Flop", "Q"),
    ("Enable Gate", "Out"),
    ("Parallel Decoder", "Words"),
];
const VALIDATION_OUTPUTS: [(&str, &str); 1] = [("SPI Decoder", "MOSI Bits")];

struct BenchmarkGraphExecution {
    lowerer: GraphLowerer,
    runtime: GraphRuntime,
}

impl BenchmarkGraphExecution {
    fn new(lowerer: GraphLowerer, runtime: GraphRuntime) -> Self {
        Self { lowerer, runtime }
    }

    fn lowerer(&self) -> &GraphLowerer {
        &self.lowerer
    }

    fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan) {
        self.lowerer.set_output_subscriptions(subscriptions);
    }

    fn set_artifact_repository(&mut self, repository: Arc<dyn ArtifactRepository>) {
        self.runtime.set_artifact_repository(repository);
    }

    fn start(
        &self,
        graph: ProcessingGraph,
        context: &mut GraphRunContext,
        source_overrides: SourceProcessOverrides,
    ) -> Result<LiveRun, Vec<ProcessingGraphError>> {
        self.runtime.start(graph, context, source_overrides)
    }

    fn apply(&self, run: &mut LiveRun, graph: ProcessingGraph) -> Result<ApplySummary, ApplyError> {
        self.runtime.apply(run, graph)
    }
}

struct BenchmarkOutputStorage;

struct BenchmarkArtifactRepository {
    waveform_indexes: Arc<dyn ArtifactRepository>,
    transient: MemoryArtifactRepository,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
struct ArtifactWriteProfile {
    artifacts: u64,
    bytes: u64,
    begin_write_ns: u64,
    write_ns: u64,
    truncate_ns: u64,
    flush_ns: u64,
    publish_ns: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
struct WorkTaskProfile {
    tasks: u64,
    cumulative_ns: u64,
    cpu_ns: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
struct NodeWorkProfile {
    calls: u64,
    produced_items: u64,
    sampled_calls: u64,
    sampled_wall_ns: u64,
    max_sampled_call_ns: u64,
    thread_cpu_ns: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
struct ArtifactInventory {
    artifacts: u64,
    bytes: u64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
struct FinalPublicationProfile {
    lanes: u64,
    cumulative_ns: u64,
    max_lane_ns: u64,
}

struct DerivedProfileMetrics {
    artifact_writes: Mutex<BTreeMap<String, ArtifactWriteProfile>>,
    work_tasks: Mutex<BTreeMap<String, WorkTaskProfile>>,
    node_work: Mutex<BTreeMap<String, NodeWorkProfile>>,
    final_begins: Mutex<BTreeMap<SourceIdentity, Instant>>,
    final_publication: Mutex<FinalPublicationProfile>,
}

impl DerivedProfileMetrics {
    fn new() -> Self {
        Self {
            artifact_writes: Mutex::new(BTreeMap::new()),
            work_tasks: Mutex::new(BTreeMap::new()),
            node_work: Mutex::new(BTreeMap::new()),
            final_begins: Mutex::new(BTreeMap::new()),
            final_publication: Mutex::new(FinalPublicationProfile::default()),
        }
    }

    fn reset(&self) {
        self.artifact_writes.lock().unwrap().clear();
        self.work_tasks.lock().unwrap().clear();
        self.node_work.lock().unwrap().clear();
        self.final_begins.lock().unwrap().clear();
        *self.final_publication.lock().unwrap() = FinalPublicationProfile::default();
    }

    fn record_artifact(&self, category: &str, update: impl FnOnce(&mut ArtifactWriteProfile)) {
        let mut profiles = self.artifact_writes.lock().unwrap();
        update(profiles.entry(category.to_owned()).or_default());
    }

    fn record_work_task(&self, label: &str, elapsed: Duration, cpu: Option<Duration>) {
        let mut profiles = self.work_tasks.lock().unwrap();
        let profile = profiles.entry(label.to_owned()).or_default();
        profile.tasks = profile.tasks.saturating_add(1);
        profile.cumulative_ns = profile.cumulative_ns.saturating_add(duration_ns(elapsed));
        profile.cpu_ns = profile.cpu_ns.saturating_add(cpu.map_or(0, duration_ns));
    }

    fn record_node_profile(&self, name: &str, profile: NodeWorkProfile) {
        let mut profiles = self.node_work.lock().unwrap();
        profiles.insert(name.to_owned(), profile);
    }

    fn record_final_begin(&self, identity: SourceIdentity, started: Instant) {
        self.final_begins
            .lock()
            .unwrap()
            .entry(identity)
            .or_insert(started);
    }

    fn record_final_publish(&self, identity: SourceIdentity) {
        let Some(started) = self.final_begins.lock().unwrap().remove(&identity) else {
            return;
        };
        let elapsed_ns = duration_ns(started.elapsed());
        let mut profile = self.final_publication.lock().unwrap();
        profile.lanes = profile.lanes.saturating_add(1);
        profile.cumulative_ns = profile.cumulative_ns.saturating_add(elapsed_ns);
        profile.max_lane_ns = profile.max_lane_ns.max(elapsed_ns);
    }
}

struct ProfiledArtifactRepository {
    inner: Arc<dyn ArtifactRepository>,
    metrics: Arc<DerivedProfileMetrics>,
}

impl ProfiledArtifactRepository {
    fn new(inner: Arc<dyn ArtifactRepository>, metrics: Arc<DerivedProfileMetrics>) -> Self {
        Self { inner, metrics }
    }
}

impl ArtifactRepository for ProfiledArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities {
        self.inner.capabilities()
    }

    fn namespaces(&self) -> Result<Vec<ArtifactNamespace>, RepositoryError> {
        self.inner.namespaces()
    }

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError> {
        self.inner.open(key)
    }

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError> {
        let category = derived_artifact_category(key.namespace()).to_owned();
        let identity = key.identity();
        let started = Instant::now();
        let writer = self.inner.begin_write(key)?;
        if category == "derived_index" {
            self.metrics.record_final_begin(identity, started);
        }
        self.metrics.record_artifact(&category, |profile| {
            profile.begin_write_ns = profile
                .begin_write_ns
                .saturating_add(duration_ns(started.elapsed()));
        });
        Ok(Box::new(ProfiledArtifactWriter {
            inner: writer,
            category,
            identity,
            length: 0,
            metrics: Arc::clone(&self.metrics),
        }))
    }

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError> {
        self.inner.remove(key)
    }

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError> {
        self.inner.entries(namespace)
    }
}

struct ProfiledArtifactWriter {
    inner: Box<dyn WriteArtifact>,
    category: String,
    identity: SourceIdentity,
    length: u64,
    metrics: Arc<DerivedProfileMetrics>,
}

impl WriteArtifact for ProfiledArtifactWriter {
    fn key(&self) -> &ArtifactKey {
        self.inner.key()
    }

    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError> {
        let started = Instant::now();
        let result = self.inner.write_at(offset, source);
        let elapsed = started.elapsed();
        if result.is_ok() {
            self.length = self.length.max(offset.saturating_add(source.len() as u64));
        }
        self.metrics.record_artifact(&self.category, |profile| {
            profile.write_ns = profile.write_ns.saturating_add(duration_ns(elapsed));
        });
        result
    }

    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError> {
        let started = Instant::now();
        let result = self.inner.truncate(len);
        let elapsed = started.elapsed();
        if result.is_ok() {
            self.length = len;
        }
        self.metrics.record_artifact(&self.category, |profile| {
            profile.truncate_ns = profile.truncate_ns.saturating_add(duration_ns(elapsed));
        });
        result
    }

    fn flush(&mut self) -> Result<(), RepositoryError> {
        let started = Instant::now();
        let result = self.inner.flush();
        let elapsed = started.elapsed();
        self.metrics.record_artifact(&self.category, |profile| {
            profile.flush_ns = profile.flush_ns.saturating_add(duration_ns(elapsed));
        });
        result
    }

    fn publish(self: Box<Self>) -> Result<(), RepositoryError> {
        let Self {
            inner,
            category,
            identity,
            length,
            metrics,
        } = *self;
        let started = Instant::now();
        let result = inner.publish();
        let elapsed = started.elapsed();
        metrics.record_artifact(&category, |profile| {
            profile.publish_ns = profile.publish_ns.saturating_add(duration_ns(elapsed));
            if result.is_ok() {
                profile.artifacts = profile.artifacts.saturating_add(1);
                profile.bytes = profile.bytes.saturating_add(length);
            }
        });
        if result.is_ok() && category == "derived_manifest" {
            metrics.record_final_publish(identity);
        }
        result
    }
}

fn derived_artifact_category(namespace: &ArtifactNamespace) -> &'static str {
    match namespace.as_str() {
        "derived-word-index-v1" => "derived_index",
        "derived-word-manifest-v1" => "derived_manifest",
        namespace if namespace.starts_with("derived-word-blocks-v1-") => "derived_block",
        namespace if namespace.starts_with("derived-word-segments-v1-") => "derived_segment",
        "waveform-index-root-v1" => "waveform_root",
        "waveform-index-segment-v1" => "waveform_segment",
        _ => "other",
    }
}

fn artifact_inventory(
    repository: &dyn ArtifactRepository,
) -> Result<BTreeMap<String, ArtifactInventory>, RepositoryError> {
    let mut inventory = BTreeMap::<String, ArtifactInventory>::new();
    for namespace in repository.namespaces()? {
        let category = derived_artifact_category(&namespace).to_owned();
        for metadata in repository.entries(&namespace)? {
            let entry = inventory.entry(category.clone()).or_default();
            entry.artifacts = entry.artifacts.saturating_add(1);
            entry.bytes = entry.bytes.saturating_add(metadata.length);
        }
    }
    Ok(inventory)
}

impl BenchmarkArtifactRepository {
    fn new(waveform_indexes: Arc<dyn ArtifactRepository>) -> Self {
        Self {
            waveform_indexes,
            transient: MemoryArtifactRepository::new(),
        }
    }

    fn can_read_persistent(namespace: &ArtifactNamespace) -> bool {
        matches!(
            namespace.as_str(),
            "waveform-index-root-v1"
                | "waveform-index-segment-v1"
                | "capture-raw-block-v1"
                | "growing-waveform-page-v1"
        )
    }
}

impl ArtifactRepository for BenchmarkArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities {
        self.transient.capabilities()
    }

    fn namespaces(&self) -> Result<Vec<ArtifactNamespace>, RepositoryError> {
        let mut namespaces = self.transient.namespaces()?;
        namespaces.extend(
            self.waveform_indexes
                .namespaces()?
                .into_iter()
                .filter(Self::can_read_persistent),
        );
        namespaces.sort();
        namespaces.dedup();
        Ok(namespaces)
    }

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError> {
        if let Some(artifact) = self.transient.open(key)? {
            return Ok(Some(artifact));
        }
        if Self::can_read_persistent(key.namespace()) {
            return self.waveform_indexes.open(key);
        }
        Ok(None)
    }

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError> {
        self.transient.begin_write(key)
    }

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError> {
        self.transient.remove(key)
    }

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError> {
        let mut entries = self.transient.entries(namespace)?;
        if Self::can_read_persistent(namespace) {
            entries.extend(self.waveform_indexes.entries(namespace)?);
            entries.sort_by(|left, right| left.key.cmp(&right.key));
            entries.dedup_by(|left, right| left.key == right.key);
        }
        Ok(entries)
    }
}

struct BenchmarkDslFileSourceFactory;

struct BenchmarkDslFileSourceMetadata {
    config: DslFileSourceConfig,
}

impl CaptureSourceMetadata for BenchmarkDslFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        DslFileSource::indexed_capture_presentation_from_path(self.config.path())
            .map(CaptureSourcePresentation::Indexed)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        DslFileSource::capture_cache_identity(self.config.path())
            .map(CaptureSourceCacheIdentity::Stable)
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        DslFileSource::new(self.config.path())
            .map(|source| Some(source.header().probe_names.clone()))
            .map_err(|error| error.to_string())
    }
}

impl DslFileSourceFactory for BenchmarkDslFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true)
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(BenchmarkDslFileSourceMetadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        DslFileSource::new(config.path())
            .map(|source| {
                ProcessNodeConstruction::new(
                    Box::new(
                        source
                            .with_name(name)
                            .with_artifact_repository(artifact_repository)
                            .with_work_executor(work_executor),
                    ) as Box<dyn ProcessNode>,
                    metadata,
                )
            })
            .map_err(|error| error.to_string())
    }
}

impl OutputStorage for BenchmarkOutputStorage {
    fn create_parent_dirs(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    fn create(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        std::fs::File::create(path).map(|file| Box::new(file) as Box<dyn OutputFile>)
    }

    fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(|file| Box::new(file) as Box<dyn OutputFile>)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

fn benchmark_binary_writer() -> BinaryFileWriter {
    BinaryFileWriter::with_output_storage(Arc::new(BenchmarkOutputStorage))
}
const CHECKED_IN_GRAPH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/graphs/spi_controlled_decode.json"
);

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
struct OutputFingerprint {
    name: String,
    size: u64,
    hash: String,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct ProtocolSelectionReport {
    strategy: String,
    pipeline_wall_s: f64,
    output_manifest: Vec<OutputFingerprint>,
    derived_lane_rows: u64,
    derived_lane_fingerprint: String,
}

#[derive(Clone, Copy, Debug)]
struct ResourceUsage {
    user_seconds: f64,
    system_seconds: f64,
    peak_rss_bytes: u64,
}

struct BenchmarkWorkExecutor;

impl WorkExecutor for BenchmarkWorkExecutor {
    fn available_parallelism(&self) -> usize {
        1
    }

    fn supports_long_running_tasks(&self) -> bool {
        true
    }

    fn idle(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        let handle = std::thread::Builder::new()
            .name("compiler-capture-runtime".into())
            .spawn(task)
            .map_err(|error| error.to_string())?;
        Ok(Box::new(BenchmarkWorkTask {
            handle: Some(handle),
        }))
    }
}

struct ProfileWorkExecutor {
    workers: usize,
}

impl ProfileWorkExecutor {
    fn new(workers: usize) -> Self {
        assert!(workers > 0);
        Self { workers }
    }
}

impl WorkExecutor for ProfileWorkExecutor {
    fn available_parallelism(&self) -> usize {
        self.workers
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        let handle = std::thread::Builder::new()
            .name("waveform-index-profile".into())
            .spawn(task)
            .map_err(|error| error.to_string())?;
        Ok(Box::new(BenchmarkWorkTask {
            handle: Some(handle),
        }))
    }
}

struct ProfiledWorkExecutor {
    inner: Arc<dyn WorkExecutor>,
    metrics: Arc<DerivedProfileMetrics>,
}

impl ProfiledWorkExecutor {
    fn new(inner: Arc<dyn WorkExecutor>, metrics: Arc<DerivedProfileMetrics>) -> Self {
        Self { inner, metrics }
    }
}

impl WorkExecutor for ProfiledWorkExecutor {
    fn available_parallelism(&self) -> usize {
        self.inner.available_parallelism()
    }

    fn supports_long_running_tasks(&self) -> bool {
        self.inner.supports_long_running_tasks()
    }

    fn idle(&self, duration: Duration) {
        self.inner.idle(duration);
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        self.submit_labeled("unlabeled", task)
    }

    fn submit_labeled(
        &self,
        label: &'static str,
        task: WorkExecutorTask,
    ) -> Result<Box<dyn WorkTask>, String> {
        let metrics = Arc::clone(&self.metrics);
        self.inner.submit_labeled(
            label,
            Box::new(move || {
                let cpu_started = thread_cpu_time();
                let started = Instant::now();
                task();
                let cpu = duration_delta(cpu_started, thread_cpu_time());
                metrics.record_work_task(label, started.elapsed(), cpu);
            }),
        )
    }

    fn submit_long_running(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        self.submit_long_running_labeled("unlabeled-long-running", task)
    }

    fn submit_long_running_labeled(
        &self,
        label: &'static str,
        task: WorkExecutorTask,
    ) -> Result<Box<dyn WorkTask>, String> {
        let metrics = Arc::clone(&self.metrics);
        self.inner.submit_long_running_labeled(
            label,
            Box::new(move || {
                let cpu_started = thread_cpu_time();
                let started = Instant::now();
                task();
                let cpu = duration_delta(cpu_started, thread_cpu_time());
                metrics.record_work_task(label, started.elapsed(), cpu);
            }),
        )
    }
}

struct BenchmarkWorkTask {
    handle: Option<JoinHandle<()>>,
}

struct BenchmarkAppManagerFactory {
    work_executor: Arc<dyn WorkExecutor>,
    metrics: Option<Arc<DerivedProfileMetrics>>,
}

impl AppManagerFactory for BenchmarkAppManagerFactory {
    fn create(&self) -> AppManager {
        AppManager::with_backend(Box::new(BenchmarkAppManagerBackend {
            manager: PipelineManager::new(Arc::clone(&self.work_executor)),
            metrics: self.metrics.clone(),
        }))
    }
}

struct BenchmarkAppManagerBackend {
    manager: PipelineManager,
    metrics: Option<Arc<DerivedProfileMetrics>>,
}

impl AppManagerBackend for BenchmarkAppManagerBackend {
    fn is_finished(&self) -> bool {
        self.manager.is_finished()
    }

    fn add_node(&mut self, spec: NodeSpec) -> Result<(), String> {
        self.manager
            .add_node(profile_node_spec(spec, &self.metrics))
    }

    fn add_node_deferred(&mut self, spec: NodeSpec) -> Result<(), String> {
        self.manager
            .add_node_deferred(profile_node_spec(spec, &self.metrics))
    }

    fn start_all_deferred(&mut self) -> Result<(), String> {
        self.manager.start_all_deferred()
    }

    fn remove_node(&mut self, name: &str) -> Result<(), String> {
        self.manager.remove_node(name)
    }

    fn reconfigure(&mut self, name: &str, config: NodeConfig) -> Result<(), String> {
        self.manager.reconfigure(name, config)
    }

    fn reconfigure_at(
        &mut self,
        name: &str,
        config: NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> Result<(), String> {
        self.manager.reconfigure_at(name, config, boundary)
    }

    fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn ProcessNode>,
        inputs: Vec<Option<InputSub>>,
    ) -> Result<(), String> {
        let node = if let Some(metrics) = &self.metrics {
            Box::new(TimedProcessNode {
                name: name.to_owned(),
                inner: node,
                metrics: Arc::clone(metrics),
                profile: NodeWorkProfile::default(),
                first_cpu: None,
            }) as Box<dyn ProcessNode>
        } else {
            node
        };
        self.manager.restart_node(name, node, inputs)
    }

    fn progress(&self) -> Vec<(String, u64)> {
        self.manager.progress()
    }

    fn take_disconnected(&self) -> Vec<DisconnectEvent> {
        self.manager.take_disconnected()
    }

    fn request_stop(&mut self) {
        self.manager.request_stop();
    }

    fn wait(&mut self) {
        self.manager.wait();
    }

    fn pump(&mut self, budget: usize) {
        self.manager.pump(budget);
    }
}

fn profile_node_spec(mut spec: NodeSpec, metrics: &Option<Arc<DerivedProfileMetrics>>) -> NodeSpec {
    if let Some(metrics) = metrics {
        spec.node = Box::new(TimedProcessNode {
            name: spec.name.clone(),
            inner: spec.node,
            metrics: Arc::clone(metrics),
            profile: NodeWorkProfile::default(),
            first_cpu: None,
        });
    }
    spec
}

struct TimedProcessNode {
    name: String,
    inner: Box<dyn ProcessNode>,
    metrics: Arc<DerivedProfileMetrics>,
    profile: NodeWorkProfile,
    first_cpu: Option<Duration>,
}

impl ProcessNode for TimedProcessNode {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn should_stop(&self) -> bool {
        self.inner.should_stop()
    }

    fn is_self_threading(&self) -> bool {
        self.inner.is_self_threading()
    }

    fn set_runtime_execution_mode(&mut self, mode: signal_runtime::RuntimeExecutionMode) {
        self.inner.set_runtime_execution_mode(mode);
    }

    fn input_scheduling(&self) -> signal_runtime::InputScheduling {
        self.inner.input_scheduling()
    }

    fn num_inputs(&self) -> usize {
        self.inner.num_inputs()
    }

    fn num_outputs(&self) -> usize {
        self.inner.num_outputs()
    }

    fn input_schema(&self) -> Vec<signal_runtime::PortSchema> {
        self.inner.input_schema()
    }

    fn output_schema(&self) -> Vec<signal_runtime::PortSchema> {
        self.inner.output_schema()
    }

    fn select_input_protocols(
        &self,
        candidates: &[Option<signal_runtime::InputProtocolCandidate>],
    ) -> Vec<Option<signal_runtime::ProtocolKind>> {
        self.inner.select_input_protocols(candidates)
    }

    fn node_type(&self) -> &str {
        self.inner.node_type()
    }

    fn work(
        &mut self,
        inputs: &[signal_runtime::InputPort],
        outputs: &[signal_runtime::OutputPort],
    ) -> signal_runtime::WorkResult<usize> {
        self.inner.work(inputs, outputs)
    }

    fn work_outcome(
        &mut self,
        inputs: &[signal_runtime::InputPort],
        outputs: &[signal_runtime::OutputPort],
    ) -> signal_runtime::WorkResult<signal_runtime::WorkOutcome> {
        let sample = self.profile.calls == 0 || self.profile.calls.is_multiple_of(1_024);
        let started = sample.then(Instant::now);
        let result = self.inner.work_outcome(inputs, outputs);
        self.profile.calls = self.profile.calls.saturating_add(1);
        self.profile.produced_items = self.profile.produced_items.saturating_add(
            result
                .as_ref()
                .ok()
                .map_or(0, |outcome| outcome.produced_items() as u64),
        );
        if let Some(started) = started {
            let elapsed_ns = duration_ns(started.elapsed());
            self.profile.sampled_calls = self.profile.sampled_calls.saturating_add(1);
            self.profile.sampled_wall_ns = self.profile.sampled_wall_ns.saturating_add(elapsed_ns);
            self.profile.max_sampled_call_ns = self.profile.max_sampled_call_ns.max(elapsed_ns);
            let cpu = thread_cpu_time();
            let first = *self.first_cpu.get_or_insert(cpu.unwrap_or_default());
            self.profile.thread_cpu_ns = cpu.map_or(self.profile.thread_cpu_ns, |cpu| {
                duration_ns(cpu.saturating_sub(first))
            });
        }
        result
    }

    fn apply_config(&mut self, config: &NodeConfig) -> signal_runtime::ConfigOutcome {
        self.inner.apply_config(config)
    }

    fn configuration_scheduler(&self) -> Option<Arc<dyn signal_runtime::ConfigurationScheduler>> {
        self.inner.configuration_scheduler()
    }

    fn cancellation(&self) -> Option<Arc<dyn signal_runtime::NodeCancellation>> {
        self.inner.cancellation()
    }

    fn protocol_capability(
        &self,
        port: usize,
        protocol: signal_runtime::ProtocolKind,
        input_capabilities: &[Vec<signal_runtime::ProtocolCapability>],
    ) -> Option<signal_runtime::ProtocolCapability> {
        self.inner
            .protocol_capability(port, protocol, input_capabilities)
    }
}

impl Drop for TimedProcessNode {
    fn drop(&mut self) {
        self.metrics
            .record_node_profile(&self.name, self.profile.clone());
    }
}

impl WorkTask for BenchmarkWorkTask {
    fn is_finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn wait(mut self: Box<Self>) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn runtime_executor() -> Arc<dyn WorkExecutor> {
    Arc::new(BenchmarkWorkExecutor)
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(unix)]
fn thread_cpu_time() -> Option<Duration> {
    let mut value = std::mem::MaybeUninit::<libc::timespec>::zeroed();
    let status = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, value.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    let value = unsafe { value.assume_init() };
    Some(Duration::new(
        u64::try_from(value.tv_sec).ok()?,
        u32::try_from(value.tv_nsec).ok()?,
    ))
}

#[cfg(not(unix))]
fn thread_cpu_time() -> Option<Duration> {
    None
}

fn duration_delta(before: Option<Duration>, after: Option<Duration>) -> Option<Duration> {
    before
        .zip(after)
        .map(|(before, after)| after.saturating_sub(before))
}

#[cfg(unix)]
fn resource_usage() -> Option<ResourceUsage> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` points to writable storage for one `libc::rusage`,
    // which `getrusage` initializes when it succeeds.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the successful call above initialized the value.
    let usage = unsafe { usage.assume_init() };
    let timeval_seconds =
        |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1_000_000.0;
    #[cfg(target_os = "macos")]
    let peak_rss_bytes = usage.ru_maxrss.max(0) as u64;
    #[cfg(not(target_os = "macos"))]
    let peak_rss_bytes = (usage.ru_maxrss.max(0) as u64).saturating_mul(1024);
    Some(ResourceUsage {
        user_seconds: timeval_seconds(usage.ru_utime),
        system_seconds: timeval_seconds(usage.ru_stime),
        peak_rss_bytes,
    })
}

#[cfg(not(unix))]
fn resource_usage() -> Option<ResourceUsage> {
    None
}

fn resource_delta(
    before: Option<ResourceUsage>,
    after: Option<ResourceUsage>,
    wall: Duration,
) -> serde_json::Value {
    let (Some(before), Some(after)) = (before, after) else {
        return serde_json::json!({
            "cpu_user_s": null,
            "cpu_system_s": null,
            "average_cpu_cores": null,
            "peak_rss_bytes": null,
        });
    };
    let user_seconds = (after.user_seconds - before.user_seconds).max(0.0);
    let system_seconds = (after.system_seconds - before.system_seconds).max(0.0);
    let average_cpu_cores = (user_seconds + system_seconds) / wall.as_secs_f64().max(f64::EPSILON);
    serde_json::json!({
        "cpu_user_s": user_seconds,
        "cpu_system_s": system_seconds,
        "average_cpu_cores": average_cpu_cores,
        "peak_rss_bytes": after.peak_rss_bytes,
    })
}

fn startup_widget() -> NodeGraphWidget {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    let graph = std::fs::read_to_string(CHECKED_IN_GRAPH)
        .expect("checked-in SPI controlled decode graph should be readable");
    widget.set_graph(
        serde_json::from_str(&graph).expect("checked-in SPI controlled decode graph should load"),
    );
    widget
}

fn startup_output_subscriptions(widget: &NodeGraphWidget) -> OutputSubscriptionPlan {
    output_subscriptions(widget, &STARTUP_OUTPUTS)
}

fn output_subscriptions(
    widget: &NodeGraphWidget,
    outputs: &[(&str, &str)],
) -> OutputSubscriptionPlan {
    outputs
        .iter()
        .map(|&(node_title, output_name)| {
            let node = widget
                .graph()
                .nodes
                .values()
                .find(|node| node.title == node_title)
                .unwrap_or_else(|| panic!("missing node '{node_title}'"));
            let output = node
                .outputs
                .iter()
                .position(|output| output.name == output_name)
                .unwrap_or_else(|| panic!("missing output '{node_title}.{output_name}'"));
            (node.id, output)
        })
        .collect()
}

fn configured_compiler(widget: &NodeGraphWidget) -> BenchmarkGraphExecution {
    let mut compiler = BenchmarkGraphExecution::new(GraphLowerer::new(), GraphRuntime::new());
    compiler.set_output_subscriptions(startup_output_subscriptions(widget));
    compiler
}

fn configured_platform_compiler(
    widget: &NodeGraphWidget,
    artifact_repository: Arc<dyn ArtifactRepository>,
    work_executor: Arc<dyn WorkExecutor>,
) -> BenchmarkGraphExecution {
    let repository: Arc<dyn ArtifactRepository> =
        Arc::new(BenchmarkArtifactRepository::new(artifact_repository));
    let mut compiler = BenchmarkGraphExecution::new(
        GraphLowerer::with_capability_overrides(vec![
            logic_analyzer_graph_nodes::binary_file_writer_capability_override(
                logic_analyzer_processing::nodes::sinks::binary_file_writer::writer_factory(
                    Arc::new(BenchmarkOutputStorage),
                ),
            ),
            logic_analyzer_graph_nodes::dsl_file_source_capability_override(Arc::new(
                BenchmarkDslFileSourceFactory,
            )),
        ]),
        GraphRuntime::with_execution(
            Box::new(InlineSourcePreparationExecutor),
            Arc::new(BenchmarkAppManagerFactory {
                work_executor: runtime_executor(),
                metrics: None,
            }),
            work_executor,
        ),
    );
    compiler.set_artifact_repository(repository);
    compiler.set_output_subscriptions(startup_output_subscriptions(widget));
    compiler
}

fn configured_profile_compiler(
    widget: &NodeGraphWidget,
    repository: Arc<dyn ArtifactRepository>,
    work_executor: Arc<dyn WorkExecutor>,
    metrics: Arc<DerivedProfileMetrics>,
) -> BenchmarkGraphExecution {
    let mut compiler = BenchmarkGraphExecution::new(
        GraphLowerer::with_capability_overrides(vec![
            logic_analyzer_graph_nodes::binary_file_writer_capability_override(
                logic_analyzer_processing::nodes::sinks::binary_file_writer::writer_factory(
                    Arc::new(BenchmarkOutputStorage),
                ),
            ),
            logic_analyzer_graph_nodes::dsl_file_source_capability_override(Arc::new(
                BenchmarkDslFileSourceFactory,
            )),
        ]),
        GraphRuntime::with_execution(
            Box::new(InlineSourcePreparationExecutor),
            Arc::new(BenchmarkAppManagerFactory {
                work_executor: runtime_executor(),
                metrics: Some(metrics),
            }),
            work_executor,
        ),
    );
    compiler.set_artifact_repository(repository);
    compiler.set_output_subscriptions(startup_output_subscriptions(widget));
    compiler
}

fn validation_compiler(widget: &NodeGraphWidget) -> BenchmarkGraphExecution {
    let mut compiler = BenchmarkGraphExecution::new(GraphLowerer::new(), GraphRuntime::new());
    compiler.set_output_subscriptions(output_subscriptions(widget, &VALIDATION_OUTPUTS));
    compiler
}

fn protocol_selection_compiler(widget: &NodeGraphWidget) -> BenchmarkGraphExecution {
    let mut compiler = BenchmarkGraphExecution::new(GraphLowerer::new(), GraphRuntime::new());
    compiler.set_output_subscriptions(output_subscriptions(
        widget,
        &[("Parallel Decoder", "Words")],
    ));
    compiler
}

fn node_by_definition(widget: &NodeGraphWidget, definition: &str) -> NodeId {
    widget
        .graph()
        .nodes
        .values()
        .find(|node| node.def_name() == definition)
        .map(|node| node.id)
        .unwrap_or_else(|| panic!("missing node definition '{definition}'"))
}

fn configured_widget(capture: &Path, output: &Path) -> NodeGraphWidget {
    let mut widget = startup_widget();
    let source = node_by_definition(&widget, "DSL File Source");
    let formatter = node_by_definition(&widget, "String Formatter");

    let mut source_state = widget.graph().nodes[&source].state.clone();
    source_state["file"]["value"] = capture.display().to_string().into();
    assert!(widget.set_node_state(source, source_state));

    let mut formatter_state = widget.graph().nodes[&formatter].state.clone();
    formatter_state["template"]["value"] =
        format!("{}/capture_{{n:04}}.bin", output.display()).into();
    assert!(widget.set_node_state(formatter, formatter_state));
    widget
}

fn configured_widget_with_parallel_strategy(
    capture: &Path,
    output: &Path,
    strategy: ParallelInputStrategy,
) -> NodeGraphWidget {
    let mut widget = configured_widget(capture, output);
    let decoder = node_by_definition(&widget, "Parallel Decoder");
    let mut state = widget.graph().nodes[&decoder].state.clone();
    state["input_strategy"]["value"] = match strategy {
        ParallelInputStrategy::Auto => "Auto",
        ParallelInputStrategy::PackedStream => "Packed stream",
        ParallelInputStrategy::Indexed => "Indexed",
    }
    .into();
    assert!(widget.set_node_state(decoder, state));
    widget
}

fn attach_matcher_tap(widget: &mut NodeGraphWidget) -> (NodeId, usize) {
    let matcher = widget
        .add_node_at("Word Matcher", Pos2::new(620.0, 600.0))
        .expect("Word Matcher is registered");
    let mut state = widget.graph().nodes[&matcher].state.clone();
    state["pattern"]["value"] = "0x0".into();
    state["mask"]["value"] = "0x0".into();
    assert!(widget.set_node_state(matcher, state));

    let decoder = node_by_definition(widget, "Parallel Decoder");
    let decoder_words = widget.graph().nodes[&decoder]
        .outputs
        .iter()
        .position(|socket| socket.name == "Words")
        .expect("Parallel Decoder has Words output");
    let matcher_input = widget.graph().nodes[&matcher]
        .inputs
        .iter()
        .position(|socket| socket.name == "Words" && socket.visible)
        .expect("Word Matcher has Words input");
    let matcher_output = widget.graph().nodes[&matcher]
        .outputs
        .iter()
        .position(|socket| socket.name == "Match")
        .expect("Word Matcher has Match output");
    widget.graph_mut().add_connection(
        SocketId {
            node: decoder,
            index: decoder_words,
            direction: SocketDirection::Output,
        },
        SocketId {
            node: matcher,
            index: matcher_input,
            direction: SocketDirection::Input,
        },
    );
    (matcher, matcher_output)
}

fn run_phase_one_reference(capture: &Path, output: &Path) {
    let mut pipeline = Pipeline::new().with_default_buffer_size(10_000_000);
    pipeline
        .add_process(
            "source",
            DslFileSource::new(capture)
                .unwrap()
                .with_work_executor(runtime_executor()),
        )
        .unwrap();
    pipeline
        .add_process("spi", SpiDecoder::new(SpiMode::Mode0, 24, true, false))
        .unwrap();
    pipeline
        .add_process("start", WordMatcher::new(0x600081, u64::MAX))
        .unwrap();
    pipeline
        .add_process("stop", WordMatcher::new(0x600000, u64::MAX))
        .unwrap();
    pipeline.add_process("latch", SrLatch::new(false)).unwrap();
    pipeline
        .add_process("counter", TriggerCounter::new(0, 1))
        .unwrap();
    pipeline
        .add_process(
            "formatter",
            TextFormatter::new(format!("{}/capture_{{n:04}}.bin", output.display())),
        )
        .unwrap();
    pipeline
        .add_process(
            "decoder",
            ParallelDecoder::new(8, StrobeMode::AnyEdge, CsPolarity::ActiveLow),
        )
        .unwrap();
    pipeline
        .add_process("writer", benchmark_binary_writer().with_index_csv(true))
        .unwrap();

    pipeline.connect("source", "ch7", "spi", "clk").unwrap();
    pipeline.connect("source", "ch8", "spi", "cs").unwrap();
    pipeline.connect("source", "ch6", "spi", "mosi").unwrap();
    pipeline
        .connect("spi", "mosi_words", "start", "words")
        .unwrap();
    pipeline
        .connect("spi", "mosi_words", "stop", "words")
        .unwrap();
    pipeline
        .connect("start", "trigger", "latch", "set")
        .unwrap();
    pipeline
        .connect("stop", "trigger", "latch", "reset")
        .unwrap();
    pipeline
        .connect("latch", "q", "decoder", "enable_signal")
        .unwrap();
    pipeline
        .connect("start", "trigger", "counter", "trigger")
        .unwrap();
    pipeline
        .connect("counter", "count", "formatter", "value")
        .unwrap();
    pipeline
        .connect("formatter", "text", "writer", "filename")
        .unwrap();
    connect_parallel_inputs(&mut pipeline, "decoder");
    pipeline.connect("source", "ch8", "decoder", "cs").unwrap();
    pipeline
        .connect("decoder", "words", "writer", "data")
        .unwrap();
    pipeline.build(runtime_executor()).unwrap().wait();
}

fn run_current_reference(capture: &Path, output: &Path) {
    let mut pipeline = Pipeline::new().with_default_buffer_size(10_000_000);
    pipeline
        .add_process(
            "source",
            DslFileSource::new(capture)
                .unwrap()
                .with_work_executor(runtime_executor()),
        )
        .unwrap();
    pipeline
        .add_process("spi", SpiDecoder::new(SpiMode::Mode0, 24, true, false))
        .unwrap();
    pipeline
        .add_process("start", WordMatcher::new(0x600081, u64::MAX))
        .unwrap();
    pipeline
        .add_process("stop", WordMatcher::new(0x600000, u64::MAX))
        .unwrap();
    pipeline.add_process("latch", SrLatch::new(false)).unwrap();
    pipeline
        .add_process("gate", LogicGate::new(GateOp::And, 2))
        .unwrap();
    pipeline
        .add_process("counter", TriggerCounter::new(0, 1))
        .unwrap();
    pipeline
        .add_process(
            "formatter",
            TextFormatter::new(format!("{}/capture_{{n:04}}.bin", output.display())),
        )
        .unwrap();
    pipeline
        .add_process(
            "decoder",
            ParallelDecoder::new(8, StrobeMode::AnyEdge, CsPolarity::Disabled),
        )
        .unwrap();
    pipeline
        .add_process("writer", benchmark_binary_writer().with_index_csv(true))
        .unwrap();

    pipeline.connect("source", "ch7", "spi", "clk").unwrap();
    pipeline.connect("source", "ch8", "spi", "cs").unwrap();
    pipeline.connect("source", "ch6", "spi", "mosi").unwrap();
    pipeline
        .connect("spi", "mosi_words", "start", "words")
        .unwrap();
    pipeline
        .connect("spi", "mosi_words", "stop", "words")
        .unwrap();
    pipeline
        .connect("start", "trigger", "latch", "set")
        .unwrap();
    pipeline
        .connect("stop", "trigger", "latch", "reset")
        .unwrap();
    pipeline.connect("source", "ch8", "gate", "in0").unwrap();
    pipeline.connect("latch", "q", "gate", "in1").unwrap();
    pipeline
        .connect("gate", "out", "decoder", "enable_signal")
        .unwrap();
    pipeline
        .connect("start", "trigger", "counter", "trigger")
        .unwrap();
    pipeline
        .connect("counter", "count", "formatter", "value")
        .unwrap();
    pipeline
        .connect("formatter", "text", "writer", "filename")
        .unwrap();
    connect_parallel_inputs(&mut pipeline, "decoder");
    pipeline
        .connect("decoder", "words", "writer", "data")
        .unwrap();
    pipeline.build(runtime_executor()).unwrap().wait();
}

fn connect_parallel_inputs(pipeline: &mut Pipeline, decoder: &str) {
    pipeline
        .connect("source", "ch10", decoder, "strobe")
        .unwrap();
    for bit in 0..8 {
        pipeline
            .connect("source", &format!("ch{bit}"), decoder, &format!("d{bit}"))
            .unwrap();
    }
}

fn temporary_workspace() -> tempfile::TempDir {
    let workspace = tempfile::Builder::new()
        .prefix("logic-conduit-compiler-capture-")
        .tempdir()
        .expect("temporary validation workspace should be available");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .expect("repository root should be available");
    let workspace_path = workspace
        .path()
        .canonicalize()
        .expect("temporary validation workspace should be available");
    assert!(
        !workspace_path.starts_with(&repository),
        "temporary validation workspace '{}' must be outside the repository",
        workspace_path.display()
    );
    workspace
}

fn compile_context(_workspace: &Path) -> GraphRunContext {
    GraphRunContext::default()
}

fn binary_files(directory: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            (name.starts_with("capture_") && name.ends_with(".bin")).then_some(name)
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn normalized_csv(directory: &Path) -> Vec<u8> {
    std::fs::read_to_string(directory.join("captures.csv"))
        .unwrap()
        .lines()
        .map(|line| {
            line.split(',')
                .map(|field| field.rsplit(['/', '\\']).next().unwrap_or(field))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn fingerprint_file(path: &Path) -> (u64, String) {
    let mut file = std::fs::File::open(path).expect("output file should be readable");
    let size = file
        .metadata()
        .expect("output metadata should be readable")
        .len();
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .expect("output file should remain readable");
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    (size, hasher.finalize().to_hex().to_string())
}

fn output_manifest(directory: &Path) -> Vec<OutputFingerprint> {
    let mut entries = std::fs::read_dir(directory)
        .expect("output directory should be readable")
        .filter_map(|entry| {
            let entry = entry.expect("output entry should be readable");
            let file_type = entry
                .file_type()
                .expect("output entry type should be readable");
            file_type.is_file().then_some(entry)
        })
        .map(|entry| {
            let name = entry
                .file_name()
                .into_string()
                .expect("output names should be UTF-8");
            let (size, hash) = if name == "captures.csv" {
                let bytes = normalized_csv(directory);
                (
                    bytes.len() as u64,
                    blake3::hash(&bytes).to_hex().to_string(),
                )
            } else {
                fingerprint_file(&entry.path())
            };
            OutputFingerprint { name, size, hash }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

fn manifest_fingerprint(manifest: &[OutputFingerprint]) -> String {
    let mut hasher = blake3::Hasher::new();
    for output in manifest {
        for field in [output.name.as_bytes(), output.hash.as_bytes()] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        hasher.update(&output.size.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn parallel_lane_fingerprint(run: &logic_analyzer_graph_runtime::LiveRun) -> (u64, String) {
    let lane = run
        .derived_lanes()
        .opaque_lanes()
        .into_iter()
        .find(|lane| lane.name() == "Parallel Decoder.Words")
        .expect("protocol-selection benchmark must collect Parallel Decoder.Words");
    let query = lane
        .query::<CollectedWordLaneQuery>()
        .expect("Parallel Decoder.Words must use the built-in indexed word adapter");
    let indexed = query
        .indexed_lane()
        .expect("protocol-selection benchmark requires an indexed derived lane");
    let metadata = indexed.metadata();
    assert!(
        !metadata.is_live,
        "derived lane must be complete before hashing"
    );
    let fingerprint = indexed
        .store()
        .committed_data_fingerprint()
        .expect("complete derived lane blocks must be fingerprintable");
    (
        metadata.total_word_count,
        fingerprint
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn output_manifest_report(manifest: &[OutputFingerprint]) -> serde_json::Value {
    let total_bytes = manifest
        .iter()
        .map(|output| output.size)
        .fold(0_u64, u64::saturating_add);
    serde_json::json!({
        "fingerprint": manifest_fingerprint(manifest),
        "total_bytes": total_bytes,
        "files": manifest
            .iter()
            .map(|output| serde_json::json!({
                "name": output.name,
                "size": output.size,
                "blake3": output.hash,
            }))
            .collect::<Vec<_>>(),
    })
}

fn cache_key_hex(key: &[u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn derived_storage_report(run: &logic_analyzer_graph_runtime::LiveRun) -> serde_json::Value {
    let mut backing_counts = BTreeMap::<String, u64>::new();
    let mut retained_items = 0_u64;
    let mut resident_bytes = 0_u64;
    let mut stored_bytes = 0_u64;
    let mut index_items = 0_u64;
    let mut index_bytes = 0_u64;
    let lanes = run
        .derived_lanes()
        .opaque_lanes()
        .into_iter()
        .map(|lane| {
            let storage = lane.storage_snapshot();
            *backing_counts
                .entry(format!("{:?}", storage.backing))
                .or_default() += 1;
            retained_items = retained_items.saturating_add(storage.retained_items.unwrap_or(0));
            resident_bytes = resident_bytes.saturating_add(storage.resident_bytes.unwrap_or(0));
            stored_bytes = stored_bytes.saturating_add(storage.stored_bytes.unwrap_or(0));
            index_items = index_items.saturating_add(storage.index_items.unwrap_or(0));
            index_bytes = index_bytes.saturating_add(storage.index_bytes.unwrap_or(0));
            serde_json::json!({
                "name": lane.name(),
                "payload": lane.payload().stable_id(),
                "backing": format!("{:?}", storage.backing),
                "retained_items": storage.retained_items,
                "resident_bytes": storage.resident_bytes,
                "stored_bytes": storage.stored_bytes,
                "index_items": storage.index_items,
                "index_bytes": storage.index_bytes,
            })
        })
        .collect::<Vec<_>>();

    let mut seen = HashSet::new();
    let mut persistent_data_bytes = 0_u64;
    let mut persistent_index_bytes = 0_u64;
    let mut persistent_total_bytes = 0_u64;
    let mut persistent_words = 0_u64;
    let caches = run
        .persistent_cache_configs()
        .into_iter()
        .filter(|config| seen.insert(config.cache_key))
        .map(|config| {
            let inspected = signal_derived::derived_word_store::inspect_cache_entry(&config)
                .unwrap_or_else(|error| panic!("could not inspect derived cache: {error}"));
            if let Some(snapshot) = inspected {
                persistent_data_bytes = persistent_data_bytes.saturating_add(snapshot.data_bytes);
                persistent_index_bytes =
                    persistent_index_bytes.saturating_add(snapshot.index_bytes);
                persistent_total_bytes =
                    persistent_total_bytes.saturating_add(snapshot.total_bytes);
                persistent_words = persistent_words.saturating_add(snapshot.word_count);
                serde_json::json!({
                    "cache_key": cache_key_hex(&config.cache_key),
                    "total_bytes": snapshot.total_bytes,
                    "data_bytes": snapshot.data_bytes,
                    "index_bytes": snapshot.index_bytes,
                    "word_count": snapshot.word_count,
                    "block_count": snapshot.block_count,
                })
            } else {
                serde_json::json!({
                    "cache_key": cache_key_hex(&config.cache_key),
                    "missing": true,
                })
            }
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "lane_count": lanes.len(),
        "backing_counts": backing_counts,
        "retained_items": retained_items,
        "resident_bytes": resident_bytes,
        "stored_bytes": stored_bytes,
        "index_items": index_items,
        "index_bytes": index_bytes,
        "lanes": lanes,
        "persistent_cache": {
            "entry_count": caches.len(),
            "word_count": persistent_words,
            "data_bytes": persistent_data_bytes,
            "index_bytes": persistent_index_bytes,
            "total_bytes": persistent_total_bytes,
            "entries": caches,
        },
    })
}

fn print_manifest(manifest: &[OutputFingerprint]) {
    eprintln!("validated output manifest:");
    for output in manifest {
        eprintln!(
            "  name={} size={} blake3={}",
            output.name, output.size, output.hash
        );
    }
}

fn assert_outputs_equal(actual: &Path, expected: &Path) {
    let actual_manifest = output_manifest(actual);
    let expected_manifest = output_manifest(expected);
    assert!(
        expected_manifest
            .iter()
            .any(|output| output.name.starts_with("capture_") && output.name.ends_with(".bin")),
        "reference pipeline did not produce capture files"
    );
    let actual_names = actual_manifest
        .iter()
        .map(|output| output.name.as_str())
        .collect::<Vec<_>>();
    let expected_names = expected_manifest
        .iter()
        .map(|output| output.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual_names, expected_names, "output names differ");
    for (actual, expected) in actual_manifest.iter().zip(&expected_manifest) {
        assert_eq!(
            actual.size, expected.size,
            "output size differs for '{}'",
            expected.name
        );
        assert_eq!(
            actual.hash, expected.hash,
            "output hash differs for '{}'",
            expected.name
        );
    }
    print_manifest(&actual_manifest);
}

fn run_validation_child(stage: &str, capture: &Path, output: &Path, workspace: Option<&Path>) {
    let mut command = Command::new(
        std::env::current_exe().expect("validation executable path should be available"),
    );
    command.arg(stage).arg(capture).arg(output);
    if let Some(workspace) = workspace {
        command.arg(workspace);
    }
    let started = Instant::now();
    eprintln!("validation stage={}", stage.trim_start_matches("__"));
    let status = command
        .status()
        .expect("validation child process should start");
    assert!(
        status.success(),
        "validation stage '{}' failed with {status}",
        stage.trim_start_matches("__")
    );
    eprintln!(
        "validation stage={} elapsed={:.3}s",
        stage.trim_start_matches("__"),
        started.elapsed().as_secs_f64()
    );
}

fn run_compiled_validation_stage(capture: &Path, output: &Path, workspace: &Path) {
    let widget = configured_widget(capture, output);
    let compiler = validation_compiler(&widget);
    let mut context = compile_context(workspace);
    let lanes = context.derived_lanes().clone();
    let mut run = compiler
        .start(
            compiler.lowerer().lower(widget.graph()).unwrap(),
            &mut context,
            Default::default(),
        )
        .unwrap();
    run.wait();

    let lanes = lanes.opaque_lanes();
    for (node, output) in VALIDATION_OUTPUTS {
        let expected = format!("{node}.{output}");
        assert!(
            lanes.iter().any(|lane| lane.name() == expected),
            "compiled graph did not collect subscribed output '{expected}'"
        );
    }
    assert!(lanes.iter().any(|lane| {
        lane.payload().stable_id() == "org.logicconduit.word/v1"
            && lane
                .table_metadata()
                .is_some_and(|metadata| metadata.total_rows > 0)
    }));
}

fn run_compiled_baseline_stage(capture: &Path, output: &Path, workspace: &Path, report: &Path) {
    let total_started = Instant::now();
    let graph_started = Instant::now();
    let widget = configured_widget(capture, output);
    let compiler = configured_compiler(&widget);
    let graph_load = graph_started.elapsed();

    let lower_started = Instant::now();
    let compiled = compiler
        .lowerer()
        .lower(widget.graph())
        .unwrap_or_else(|errors| panic!("checked-in graph did not lower: {errors:?}"));
    let lower = lower_started.elapsed();
    let compiled_nodes = compiled.nodes.len();
    let compiled_edges = compiled.edges.len();

    let mut context = compile_context(workspace);
    let resources_before = resource_usage();
    let start_started = Instant::now();
    let mut run = compiler
        .start(compiled, &mut context, Default::default())
        .unwrap_or_else(|errors| panic!("checked-in graph did not start: {errors:?}"));
    let pipeline_start = start_started.elapsed();
    let execute_started = Instant::now();
    run.wait();
    let pipeline_execute = execute_started.elapsed();
    let pipeline_wall = pipeline_start.saturating_add(pipeline_execute);
    let resources = resource_delta(resources_before, resource_usage(), pipeline_wall);

    let storage_started = Instant::now();
    let storage = derived_storage_report(&run);
    let storage_inspection = storage_started.elapsed();
    let report_value = serde_json::json!({
        "schema_version": 1,
        "compiled_graph": {
            "nodes": compiled_nodes,
            "edges": compiled_edges,
        },
        "phases": {
            "graph_load_s": graph_load.as_secs_f64(),
            "graph_lower_s": lower.as_secs_f64(),
            "pipeline_start_s": pipeline_start.as_secs_f64(),
            "pipeline_execute_s": pipeline_execute.as_secs_f64(),
            "pipeline_wall_s": pipeline_wall.as_secs_f64(),
            "storage_inspection_s": storage_inspection.as_secs_f64(),
            "child_total_s": total_started.elapsed().as_secs_f64(),
        },
        "resources": resources,
        "derived_storage": storage,
    });
    std::fs::write(
        report,
        serde_json::to_vec_pretty(&report_value).expect("baseline report should serialize"),
    )
    .expect("baseline report should be writable");
}

fn run_protocol_selection_stage(
    capture: &Path,
    output: &Path,
    workspace: &Path,
    report: &Path,
    strategy: ParallelInputStrategy,
) {
    std::fs::create_dir_all(output).expect("protocol benchmark output should be available");
    let widget = configured_widget_with_parallel_strategy(capture, output, strategy);
    let compiler = protocol_selection_compiler(&widget);
    let mut context = compile_context(workspace);
    let started = Instant::now();
    let mut run = compiler
        .start(
            compiler.lowerer().lower(widget.graph()).unwrap(),
            &mut context,
            Default::default(),
        )
        .unwrap_or_else(|errors| panic!("protocol benchmark graph did not start: {errors:?}"));
    run.wait();
    let pipeline_wall = started.elapsed();
    let (derived_lane_rows, derived_lane_fingerprint) = parallel_lane_fingerprint(&run);
    let output_manifest = output_manifest(output);
    assert!(
        !output_manifest.is_empty(),
        "protocol benchmark must produce writer output"
    );
    let report_value = ProtocolSelectionReport {
        strategy: match strategy {
            ParallelInputStrategy::Auto => "auto",
            ParallelInputStrategy::PackedStream => "packed-stream",
            ParallelInputStrategy::Indexed => "indexed",
        }
        .to_owned(),
        pipeline_wall_s: pipeline_wall.as_secs_f64(),
        output_manifest,
        derived_lane_rows,
        derived_lane_fingerprint,
    };
    std::fs::write(
        report,
        serde_json::to_vec_pretty(&report_value)
            .expect("protocol-selection report should serialize"),
    )
    .expect("protocol-selection report should be writable");
}

fn run_cancellation_stage(capture: &Path, output: &Path, workspace: &Path, report: &Path) {
    let widget = configured_widget(capture, output);
    let compiler = configured_compiler(&widget);
    let mut context = compile_context(workspace);
    let mut run = compiler
        .start(
            compiler.lowerer().lower(widget.graph()).unwrap(),
            &mut context,
            Default::default(),
        )
        .unwrap_or_else(|errors| panic!("cancellation graph did not start: {errors:?}"));
    let warmup_started = Instant::now();
    while !run.is_finished()
        && !run.progress().iter().any(|(_, progress)| *progress > 0)
        && warmup_started.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        !run.is_finished(),
        "pipeline finished before cancellation probe"
    );
    let progress_before_stop = run
        .progress()
        .into_iter()
        .map(|(_, progress)| progress)
        .fold(0_u64, u64::saturating_add);
    let stop_started = Instant::now();
    run.stop();
    while !run.is_finished() && stop_started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(1));
    }
    let stop_latency = stop_started.elapsed();
    assert!(
        run.is_finished(),
        "pipeline cancellation exceeded 10 seconds"
    );
    run.wait();
    let report_value = serde_json::json!({
        "warmup_s": warmup_started.elapsed().as_secs_f64(),
        "progress_before_stop": progress_before_stop,
        "stop_latency_ms": stop_latency.as_secs_f64() * 1_000.0,
    });
    std::fs::write(
        report,
        serde_json::to_vec_pretty(&report_value).expect("cancellation report should serialize"),
    )
    .expect("cancellation report should be writable");
}

fn run_measurement_child(
    stage: &str,
    capture: &Path,
    output: &Path,
    workspace: &Path,
    report: &Path,
) {
    let started = Instant::now();
    eprintln!("baseline stage={}", stage.trim_start_matches("__"));
    let status = Command::new(
        std::env::current_exe().expect("benchmark executable path should be available"),
    )
    .arg(stage)
    .arg(capture)
    .arg(output)
    .arg(workspace)
    .arg(report)
    .status()
    .expect("benchmark child process should start");
    assert!(
        status.success(),
        "baseline stage '{}' failed with {status}",
        stage.trim_start_matches("__")
    );
    eprintln!(
        "baseline stage={} elapsed={:.3}s",
        stage.trim_start_matches("__"),
        started.elapsed().as_secs_f64()
    );
}

fn sidecar_size(path: &Path, suffix: &str) -> Option<u64> {
    let name = format!(
        "{}.{suffix}",
        path.file_name().and_then(|name| name.to_str())?
    );
    std::fs::metadata(path.with_file_name(name))
        .ok()
        .map(|metadata| metadata.len())
}

fn rustc_version() -> Option<String> {
    let output = Command::new("rustc").arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn reference_pipeline_baseline(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("compiled");
    let cancellation_workspace = workspace.path().join("cancellation");
    let cancellation_output = cancellation_workspace.join("output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::create_dir_all(&cancellation_output).unwrap();
    let child_report = workspace.path().join("baseline.json");
    let cancellation_report = workspace.path().join("cancellation.json");
    run_measurement_child(
        "__baseline",
        capture,
        &output,
        workspace.path(),
        &child_report,
    );
    run_measurement_child(
        "__cancel",
        capture,
        &cancellation_output,
        &cancellation_workspace,
        &cancellation_report,
    );

    let fingerprint_started = Instant::now();
    let manifest = output_manifest(&output);
    let output_report = output_manifest_report(&manifest);
    let fingerprint_elapsed = fingerprint_started.elapsed().as_secs_f64();
    let mut report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&child_report).expect("baseline child report should exist"),
    )
    .expect("baseline child report should be valid JSON");
    let cancellation: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&cancellation_report).expect("cancellation report should exist"),
    )
    .expect("cancellation report should be valid JSON");
    let source = DslFileSource::new(capture).expect("reference capture should open");
    let total_samples = source.total_samples();
    let samplerate_hz = source.samplerate_hz();
    let capture_seconds = total_samples as f64 / samplerate_hz;
    let pipeline_wall = report["phases"]["pipeline_wall_s"]
        .as_f64()
        .expect("pipeline wall time should be numeric");
    let graph_bytes = std::fs::read(CHECKED_IN_GRAPH).expect("checked-in graph should be readable");
    let capture_identity = DslFileSource::capture_cache_identity(capture)
        .expect("reference capture should have a stable identity");
    let generated_unix_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_secs();
    let root = report
        .as_object_mut()
        .expect("baseline report root should be an object");
    root.insert(
        "environment".to_owned(),
        serde_json::json!({
            "generated_unix_s": generated_unix_s,
            "package_version": env!("CARGO_PKG_VERSION"),
            "rustc": rustc_version(),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "logical_cpu_count": std::thread::available_parallelism().ok().map(|count| count.get()),
            "profile": "bench",
        }),
    );
    root.insert(
        "graph".to_owned(),
        serde_json::json!({
            "path": "graphs/spi_controlled_decode.json",
            "blake3": blake3::hash(&graph_bytes).to_hex().to_string(),
        }),
    );
    root.insert(
        "capture".to_owned(),
        serde_json::json!({
            "path": capture,
            "cache_identity": cache_key_hex(&capture_identity),
            "file_bytes": std::fs::metadata(capture).expect("capture metadata should be readable").len(),
            "total_samples": total_samples,
            "samplerate_hz": samplerate_hz,
            "duration_s": capture_seconds,
            "channels": source.total_probes(),
            "waveform_index_bytes": sidecar_size(capture, "idx"),
            "waveform_raw_bytes": sidecar_size(capture, "raw"),
        }),
    );
    root.insert(
        "throughput".to_owned(),
        serde_json::json!({
            "million_samples_per_s": total_samples as f64 / pipeline_wall / 1_000_000.0,
            "realtime_factor": capture_seconds / pipeline_wall,
            "target_realtime_factor": 6.0,
            "target_met": capture_seconds / pipeline_wall >= 6.0,
        }),
    );
    root.insert("outputs".to_owned(), output_report);
    root.insert("cancellation".to_owned(), cancellation);
    report["phases"]["output_fingerprint_s"] = fingerprint_elapsed.into();
    eprintln!(
        "baseline pipeline_wall_s={pipeline_wall:.3} realtime_x={:.3} output_fingerprint={}",
        capture_seconds / pipeline_wall,
        report["outputs"]["fingerprint"]
            .as_str()
            .expect("output fingerprint should be a string")
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("baseline report should serialize")
    );
}

fn waveform_index_profile(capture: &Path) {
    print_waveform_index_profile(
        capture,
        Arc::new(MemoryArtifactRepository::new()),
        logic_analyzer_platform::native_work_executor(),
        "memory",
    );
}

fn persistent_waveform_index_profile(capture: &Path) {
    let workspace = temporary_workspace();
    let repository = logic_analyzer_platform::isolated_native_artifact_repository(
        workspace.path().join("artifacts"),
    );
    print_waveform_index_profile(
        capture,
        repository,
        logic_analyzer_platform::native_work_executor(),
        "native-durable",
    );
}

fn print_waveform_index_profile(
    capture: &Path,
    repository: Arc<dyn ArtifactRepository>,
    work_executor: Arc<dyn WorkExecutor>,
    artifact_repository: &str,
) {
    let presentation = DslFileSource::indexed_capture_presentation_from_path(capture)
        .expect("reference capture should provide an indexed presentation");
    let metadata = presentation
        .factory
        .metadata()
        .expect("reference capture metadata should be readable");
    let index = presentation
        .factory
        .open(repository, work_executor, &mut |_| true)
        .expect("reference capture index should build");
    let profile = index
        .build_profile()
        .expect("fresh repository must produce an index-build profile");
    let expected_blocks = u64::try_from(metadata.total_probes)
        .ok()
        .and_then(|channels| channels.checked_mul(metadata.total_blocks))
        .expect("capture block count should fit u64");
    assert_eq!(profile.blocks, expected_blocks);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "artifact_repository": artifact_repository,
            "capture": {
                "path": capture,
                "channels": metadata.total_probes,
                "total_blocks": metadata.total_blocks,
                "samples_per_block": metadata.samples_per_block,
                "total_samples": metadata.total_samples,
            },
            "waveform_index_build": profile,
        }))
        .expect("waveform index profile should serialize")
    );
}

fn waveform_index_concurrency_profile(capture: &Path) {
    let logical_workers = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .clamp(1, 32);
    let mut worker_counts = vec![1, 2, 4, 8, 12];
    worker_counts.retain(|workers| *workers <= logical_workers);
    worker_counts.sort_unstable();
    worker_counts.dedup();

    let presentation = DslFileSource::indexed_capture_presentation_from_path(capture)
        .expect("reference capture should provide an indexed presentation");
    let metadata = presentation
        .factory
        .metadata()
        .expect("reference capture metadata should be readable");
    let mut results = Vec::with_capacity(worker_counts.len());
    for workers in worker_counts {
        let presentation = DslFileSource::indexed_capture_presentation_from_path(capture)
            .expect("reference capture should provide an indexed presentation");
        let workspace = temporary_workspace();
        let repository = logic_analyzer_platform::isolated_native_artifact_repository(
            workspace.path().join("artifacts"),
        );
        let index = presentation
            .factory
            .open(
                repository,
                Arc::new(ProfileWorkExecutor::new(workers)),
                &mut |_| true,
            )
            .expect("reference capture index should build");
        let profile = index
            .build_profile()
            .expect("fresh repository must produce an index-build profile");
        eprintln!(
            "waveform index workers={workers} wall_s={:.3}",
            profile.wall_time_ns as f64 / 1_000_000_000.0
        );
        results.push(profile);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "artifact_repository": "native-durable",
            "logical_worker_limit": logical_workers,
            "capture": {
                "path": capture,
                "channels": metadata.total_probes,
                "total_blocks": metadata.total_blocks,
                "samples_per_block": metadata.samples_per_block,
                "total_samples": metadata.total_samples,
            },
            "waveform_index_builds": results,
        }))
        .expect("waveform index concurrency profile should serialize")
    );
}

fn derived_storage_profile(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("compiled");
    let native_work_executor = logic_analyzer_platform::native_work_executor();
    let metrics = Arc::new(DerivedProfileMetrics::new());
    let native_repository = logic_analyzer_platform::isolated_native_artifact_repository(
        workspace.path().join("artifacts"),
    );
    let repository: Arc<dyn ArtifactRepository> = Arc::new(ProfiledArtifactRepository::new(
        native_repository,
        Arc::clone(&metrics),
    ));

    let presentation = DslFileSource::indexed_capture_presentation_from_path(capture)
        .expect("reference capture should provide an indexed presentation");
    presentation
        .factory
        .open(
            Arc::clone(&repository),
            Arc::clone(&native_work_executor),
            &mut |_| true,
        )
        .expect("reference capture index should prepare");
    metrics.reset();

    let work_executor: Arc<dyn WorkExecutor> = Arc::new(ProfiledWorkExecutor::new(
        native_work_executor,
        Arc::clone(&metrics),
    ));
    let widget = configured_widget(capture, &output);
    let compiler = configured_profile_compiler(
        &widget,
        Arc::clone(&repository),
        Arc::clone(&work_executor),
        Arc::clone(&metrics),
    );
    let mut context = compile_context(workspace.path());
    let resources_before = resource_usage();
    let started = Instant::now();
    let mut run = compiler
        .start(
            compiler.lowerer().lower(widget.graph()).unwrap(),
            &mut context,
            Default::default(),
        )
        .unwrap_or_else(|errors| panic!("derived storage profile did not start: {errors:?}"));
    run.wait();
    let wall = started.elapsed();
    let resources = resource_delta(resources_before, resource_usage(), wall);
    let storage = derived_storage_report(&run);
    let output_report = output_manifest_report(&output_manifest(&output));
    let artifact_writes = metrics.artifact_writes.lock().unwrap().clone();
    let work_tasks = metrics.work_tasks.lock().unwrap().clone();
    let node_work = metrics.node_work.lock().unwrap().clone();
    let final_publication = metrics.final_publication.lock().unwrap().clone();
    let retained_artifacts = artifact_inventory(repository.as_ref())
        .expect("profile artifact inventory should be readable");

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 2,
            "artifact_repository": "native-durable-isolated",
            "capture": {
                "path": capture,
                "file_bytes": std::fs::metadata(capture)
                    .expect("capture metadata should be readable")
                    .len(),
            },
            "pipeline_wall_ns": duration_ns(wall),
            "resources": resources,
            "host_work": work_tasks,
            "node_work": node_work,
            "artifact_writes": artifact_writes,
            "final_publication": final_publication,
            "retained_artifacts": retained_artifacts,
            "derived_storage": storage,
            "outputs": output_report,
        }))
        .expect("derived storage profile should serialize")
    );
}

fn in_memory_compiler_runtime_benchmark(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("compiled");
    std::fs::create_dir_all(&output).unwrap();
    let widget = configured_widget(capture, &output);
    let compiler = configured_platform_compiler(
        &widget,
        logic_analyzer_platform::native_artifact_repository("logic-conduit"),
        logic_analyzer_platform::native_work_executor(),
    );
    let mut context = compile_context(workspace.path());
    let usage_before = resource_usage();
    let started = Instant::now();
    let mut run = compiler
        .start(
            compiler.lowerer().lower(widget.graph()).unwrap(),
            &mut context,
            Default::default(),
        )
        .unwrap();
    let sampling_overlays = context.take_sampling_overlays();
    run.wait();
    let execution_elapsed = started.elapsed();
    let usage_after = resource_usage();
    let execution_cpu_seconds = usage_before
        .zip(usage_after)
        .map(|(before, after)| {
            (after.user_seconds - before.user_seconds).max(0.0)
                + (after.system_seconds - before.system_seconds).max(0.0)
        })
        .unwrap_or_default();
    eprintln!(
        "compiled graph execution: elapsed={:.3}s cpu={:.3}s average_cores={:.2}",
        execution_elapsed.as_secs_f64(),
        execution_cpu_seconds,
        execution_cpu_seconds / execution_elapsed.as_secs_f64()
    );
    let files = binary_files(&output);
    let sampling_points = sampling_overlays
        .iter()
        .find(|candidate| candidate.node_title() == "SPI Decoder")
        .expect("configured SPI sampling overlay should be available")
        .overlay()
        .points
        .points_in_range(0, 1_000_000_000)
        .len();
    let parallel_sampling = sampling_overlays
        .iter()
        .find(|candidate| candidate.node_title() == "Parallel Decoder")
        .expect("parallel sampling overlay should be available");
    let parallel_sampling_uses_retained_lane = parallel_sampling.overlay().points.has_provider();
    let first_parallel_time = run
        .derived_lanes()
        .opaque_lanes()
        .into_iter()
        .find(|lane| lane.name().contains("Parallel Decoder.Words"))
        .and_then(|lane| lane.nearest_time_boundary(0, 1_000_000_000))
        .expect("parallel word lane should expose its first timestamp");
    let visible_start = first_parallel_time.saturating_sub(100);
    let visible_end = first_parallel_time.saturating_add(10_000);
    let parallel_sampling_is_dense = parallel_sampling
        .overlay()
        .points
        .points_in_range_with_minimum_spacing(visible_start, visible_end, 1_000_000_000)
        .is_none();
    let visible_parallel_points = parallel_sampling
        .overlay()
        .points
        .points_in_range(visible_start, visible_end)
        .len();
    eprintln!(
        "compiled graph: elapsed={:.3}s overlay_queries={:.3}s files={} sampling_points={sampling_points} parallel_retained_lane={parallel_sampling_uses_retained_lane} parallel_visible_points={visible_parallel_points}",
        started.elapsed().as_secs_f64(),
        started
            .elapsed()
            .saturating_sub(execution_elapsed)
            .as_secs_f64(),
        files.len()
    );
    assert!(!files.is_empty());
    assert!(sampling_points > 0);
    assert!(parallel_sampling_uses_retained_lane);
    assert!(parallel_sampling_is_dense);
    assert!(visible_parallel_points > 0);
}

fn protocol_selection_benchmark(capture: &Path) {
    let temporary = temporary_workspace();
    let indexed_workspace = temporary.path().join("indexed-workspace");
    let packed_workspace = temporary.path().join("packed-workspace");
    let indexed_output = temporary.path().join("indexed-output");
    let packed_output = temporary.path().join("packed-output");
    let indexed_report_path = temporary.path().join("indexed-report.json");
    let packed_report_path = temporary.path().join("packed-report.json");
    run_measurement_child(
        "__protocol-indexed",
        capture,
        &indexed_output,
        &indexed_workspace,
        &indexed_report_path,
    );
    run_measurement_child(
        "__protocol-packed",
        capture,
        &packed_output,
        &packed_workspace,
        &packed_report_path,
    );

    let read_report = |path: &Path| {
        serde_json::from_slice::<ProtocolSelectionReport>(
            &std::fs::read(path).expect("protocol-selection child report should exist"),
        )
        .expect("protocol-selection child report should be valid JSON")
    };
    let indexed = read_report(&indexed_report_path);
    let packed = read_report(&packed_report_path);

    assert_eq!(indexed.strategy, "indexed");
    assert_eq!(packed.strategy, "packed-stream");
    assert_eq!(
        indexed.output_manifest, packed.output_manifest,
        "forced input protocols changed writer output"
    );
    assert_eq!(
        indexed.derived_lane_rows, packed.derived_lane_rows,
        "forced input protocols changed the derived word count"
    );
    assert_eq!(
        indexed.derived_lane_fingerprint, packed.derived_lane_fingerprint,
        "forced input protocols changed the derived lane contents"
    );

    let faster = if indexed.pipeline_wall_s <= packed.pipeline_wall_s {
        "indexed"
    } else {
        "packed-stream"
    };
    let speed_ratio = indexed.pipeline_wall_s.max(packed.pipeline_wall_s)
        / indexed.pipeline_wall_s.min(packed.pipeline_wall_s);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "graph": "graphs/spi_controlled_decode.json",
            "validation": {
                "writer_manifest": manifest_fingerprint(&indexed.output_manifest),
                "derived_lane_rows": indexed.derived_lane_rows,
                "derived_lane_fingerprint": indexed.derived_lane_fingerprint,
            },
            "throughput": {
                "indexed_wall_s": indexed.pipeline_wall_s,
                "packed_stream_wall_s": packed.pipeline_wall_s,
                "faster": faster,
                "speed_ratio": speed_ratio,
            },
        }))
        .expect("protocol-selection comparison should serialize")
    );
}

fn phase_one_reference_runtime_benchmark(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("phase-one-reference");
    std::fs::create_dir_all(&output).unwrap();
    let started = Instant::now();
    run_phase_one_reference(capture, &output);
    eprintln!(
        "phase-one reference: elapsed={:.3}s",
        started.elapsed().as_secs_f64()
    );
    assert!(!binary_files(&output).is_empty());
}

fn current_reference_runtime_benchmark(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("current-reference");
    std::fs::create_dir_all(&output).unwrap();
    let started = Instant::now();
    run_current_reference(capture, &output);
    eprintln!(
        "current reference: elapsed={:.3}s",
        started.elapsed().as_secs_f64()
    );
    assert!(!binary_files(&output).is_empty());
}

#[derive(Default)]
struct ViewerLaneTimings {
    snapshots: Mutex<Vec<Duration>>,
}

struct TimedViewerLaneQuery {
    lane: OpaqueCollectedLane,
    timings: Arc<ViewerLaneTimings>,
}

impl CollectedLaneQuery for TimedViewerLaneQuery {
    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }

    fn snapshot_generation(&self) -> Option<u64> {
        self.lane.snapshot_generation()
    }

    fn snapshot(
        &self,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        let started = Instant::now();
        let snapshot = self.lane.snapshot(request);
        self.timings
            .snapshots
            .lock()
            .unwrap()
            .push(started.elapsed());
        snapshot
    }

    fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        self.lane
            .nearest_time_boundary(timestamp_ns, max_distance_ns)
    }

    fn timeline_extent_end_ns(&self) -> Option<u64> {
        self.lane.timeline_extent_end_ns()
    }

    fn is_live(&self) -> bool {
        self.lane.is_live()
    }
}

fn live_viewer_subscription_benchmark(capture: &Path) {
    const END_NS: u64 = 250_000_000_000;
    const FRAME_INTERVAL: Duration = Duration::from_millis(16);
    const WORD_PAYLOAD_ID: &str = "org.logicconduit.word/v1";
    const WORD_RENDERER_ID: &str = "org.logicconduit.renderer.word/v1";

    fn percentile(samples: &mut [Duration], numerator: usize, denominator: usize) -> Duration {
        samples.sort_unstable();
        let index = samples.len().saturating_sub(1).saturating_mul(numerator) / denominator;
        samples.get(index).copied().unwrap_or_default()
    }

    let workspace = temporary_workspace();
    let output = workspace.path().join("compiled");
    std::fs::create_dir_all(&output).unwrap();
    let native_work_executor = logic_analyzer_platform::native_work_executor();
    let metrics = Arc::new(DerivedProfileMetrics::new());
    let native_repository = logic_analyzer_platform::isolated_native_artifact_repository(
        workspace.path().join("artifacts"),
    );
    let repository: Arc<dyn ArtifactRepository> = Arc::new(ProfiledArtifactRepository::new(
        native_repository,
        Arc::clone(&metrics),
    ));
    let presentation = DslFileSource::indexed_capture_presentation_from_path(capture)
        .expect("reference capture should provide an indexed presentation");
    presentation
        .factory
        .open(
            Arc::clone(&repository),
            Arc::clone(&native_work_executor),
            &mut |_| true,
        )
        .expect("reference capture index should prepare");
    metrics.reset();
    let work_executor: Arc<dyn WorkExecutor> = Arc::new(ProfiledWorkExecutor::new(
        native_work_executor,
        Arc::clone(&metrics),
    ));
    let widget = configured_widget(capture, &output);
    let compiler = configured_profile_compiler(
        &widget,
        Arc::clone(&repository),
        work_executor,
        Arc::clone(&metrics),
    );
    let mut context = compile_context(workspace.path());
    let resources_before = resource_usage();
    let pipeline_started = Instant::now();
    let mut run = compiler
        .start(
            compiler.lowerer().lower(widget.graph()).unwrap(),
            &mut context,
            Default::default(),
        )
        .unwrap();
    let displayed_lanes = DerivedLanes::new();
    let presentations = WaveformPresentationRegistry::new();
    presentations.register_default_payload(
        WORD_PAYLOAD_ID,
        ViewerLaneBadge::new("W", Color32::from_rgb(215, 140, 60)),
        viewer_lane_renderer(WORD_RENDERER_ID).expect("built-in word renderer is registered"),
    );
    let mut viewer = LogicAnalyzerViewer::new();
    viewer.set_channels_with_duration(Vec::new(), END_NS as f64 / 1_000.0);
    viewer.set_derived_lanes(displayed_lanes.clone());
    viewer.set_waveform_presentations(presentations);

    let screen_rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(2_560.0, 1_080.0));
    let egui_context = egui::Context::default();
    egui_context.begin_pass(egui::RawInput {
        screen_rect: Some(screen_rect),
        ..Default::default()
    });
    let mut warmup_ui = egui::Ui::new(
        egui_context.clone(),
        Id::new("live-viewer-benchmark"),
        UiBuilder::new().max_rect(screen_rect),
    );
    viewer.show(&mut warmup_ui);
    let _ = egui_context.end_pass();

    let mut timings: BTreeMap<String, Arc<ViewerLaneTimings>> = BTreeMap::new();
    let mut frame_latencies = Vec::new();
    let benchmark_started = Instant::now();
    while !run.is_finished() {
        let frame_started = Instant::now();
        for lane in run
            .derived_lanes()
            .opaque_lanes()
            .into_iter()
            .filter(|lane| lane.payload().stable_id() == WORD_PAYLOAD_ID)
        {
            if timings.contains_key(lane.name()) {
                continue;
            }
            let lane_name = lane.name().to_owned();
            let lane_timings = Arc::new(ViewerLaneTimings::default());
            displayed_lanes.publish_opaque_lane(
                lane_name.clone(),
                lane.payload().clone(),
                Arc::new(TimedViewerLaneQuery {
                    lane,
                    timings: Arc::clone(&lane_timings),
                }),
            );
            timings.insert(lane_name, lane_timings);
        }

        let lane_count = timings.len().max(1);
        let frame_index = frame_latencies.len();
        let pointer = Pos2::new(
            500.0 + (frame_index % 1_800) as f32,
            49.0 + (frame_index % lane_count) as f32 * 30.0,
        );
        egui_context.begin_pass(egui::RawInput {
            screen_rect: Some(screen_rect),
            time: Some(benchmark_started.elapsed().as_secs_f64()),
            events: vec![Event::PointerMoved(pointer)],
            ..Default::default()
        });
        let mut ui = egui::Ui::new(
            egui_context.clone(),
            Id::new("live-viewer-benchmark"),
            UiBuilder::new().max_rect(screen_rect),
        );
        viewer.show(&mut ui);
        let _ = egui_context.end_pass();

        let frame_elapsed = frame_started.elapsed();
        frame_latencies.push(frame_elapsed);
        std::thread::sleep(FRAME_INTERVAL.saturating_sub(frame_elapsed));
    }
    run.wait();
    let pipeline_wall = pipeline_started.elapsed();
    let resources = resource_delta(resources_before, resource_usage(), pipeline_wall);
    let mut lane_query_latencies = BTreeMap::new();
    for (name, timings) in timings {
        lane_query_latencies.insert(name, timings.snapshots.lock().unwrap().clone());
    }
    let mut query_latencies = lane_query_latencies
        .values()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    let query_count = query_latencies.len();
    assert!(query_count > 0);
    assert!(!binary_files(&output).is_empty());
    let frames_over_8_ms = frame_latencies
        .iter()
        .filter(|latency| **latency > Duration::from_millis(8))
        .count();
    let frames_over_16_ms = frame_latencies
        .iter()
        .filter(|latency| **latency > Duration::from_millis(16))
        .count();
    let query_p50 = percentile(&mut query_latencies.clone(), 50, 100);
    let query_p95 = percentile(&mut query_latencies.clone(), 95, 100);
    let query_p99 = percentile(&mut query_latencies, 99, 100);
    let frame_p50 = percentile(&mut frame_latencies.clone(), 50, 100);
    let frame_p95 = percentile(&mut frame_latencies.clone(), 95, 100);
    let frame_p99 = percentile(&mut frame_latencies, 99, 100);
    eprintln!(
        "live egui viewer: queries={query_count} query_p50_us={:.1} query_p95_us={:.1} query_p99_us={:.1} input_frames={} input_frame_p50_us={:.1} input_frame_p95_us={:.1} input_frame_p99_us={:.1} frames_over_8_ms={frames_over_8_ms} frames_over_16_ms={frames_over_16_ms}",
        query_p50.as_secs_f64() * 1_000_000.0,
        query_p95.as_secs_f64() * 1_000_000.0,
        query_p99.as_secs_f64() * 1_000_000.0,
        frame_latencies.len(),
        frame_p50.as_secs_f64() * 1_000_000.0,
        frame_p95.as_secs_f64() * 1_000_000.0,
        frame_p99.as_secs_f64() * 1_000_000.0,
    );
    for (lane, mut latencies) in lane_query_latencies {
        if latencies.is_empty() {
            continue;
        }
        let p50 = percentile(&mut latencies.clone(), 50, 100);
        let p95 = percentile(&mut latencies, 95, 100);
        eprintln!(
            "live viewer lane: name={lane:?} queries={} p50_us={:.1} p95_us={:.1}",
            latencies.len(),
            p50.as_secs_f64() * 1_000_000.0,
            p95.as_secs_f64() * 1_000_000.0,
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "capture": capture,
            "pipeline_wall_ns": duration_ns(pipeline_wall),
            "resources": resources,
            "viewer": {
                "queries": query_count,
                "query_p50_ns": duration_ns(query_p50),
                "query_p95_ns": duration_ns(query_p95),
                "query_p99_ns": duration_ns(query_p99),
                "input_frames": frame_latencies.len(),
                "input_frame_p50_ns": duration_ns(frame_p50),
                "input_frame_p95_ns": duration_ns(frame_p95),
                "input_frame_p99_ns": duration_ns(frame_p99),
                "frames_over_8_ms": frames_over_8_ms,
                "frames_over_16_ms": frames_over_16_ms,
            },
            "host_work": metrics.work_tasks.lock().unwrap().clone(),
            "node_work": metrics.node_work.lock().unwrap().clone(),
            "artifact_writes": metrics.artifact_writes.lock().unwrap().clone(),
            "final_publication": metrics.final_publication.lock().unwrap().clone(),
            "outputs": output_manifest_report(&output_manifest(&output)),
        }))
        .expect("responsive runtime profile should serialize")
    );
}

fn compiled_graph_matches_current_reference(capture: &Path) {
    let temporary = temporary_workspace();
    let actual = temporary.path().join("compiled");
    let expected = temporary.path().join("reference");
    std::fs::create_dir_all(&actual).unwrap();
    std::fs::create_dir_all(&expected).unwrap();
    run_validation_child("__compiled", capture, &actual, Some(temporary.path()));
    run_validation_child("__reference", capture, &expected, None);
    assert_outputs_equal(&actual, &expected);
}

fn live_attach_detach_preserves_writer_output(capture: &Path) {
    let temporary = temporary_workspace();
    let actual = temporary.path().join("compiled");
    let expected = temporary.path().join("reference");
    std::fs::create_dir_all(&actual).unwrap();
    std::fs::create_dir_all(&expected).unwrap();
    run_validation_child("__reference", capture, &expected, None);
    eprintln!("attach validation stage=compiled");
    let mut widget = configured_widget(capture, &actual);
    let mut compiler = configured_compiler(&widget);
    let base_subscriptions = startup_output_subscriptions(&widget);
    let mut context = compile_context(temporary.path());
    let lanes = context.derived_lanes().clone();
    let mut run = compiler
        .start(
            compiler.lowerer().lower(widget.graph()).unwrap(),
            &mut context,
            Default::default(),
        )
        .unwrap();

    while binary_files(&actual).is_empty() {
        assert!(!run.is_finished());
        std::thread::sleep(Duration::from_millis(200));
    }
    let (matcher, matcher_output) = attach_matcher_tap(&mut widget);
    let mut attached_subscriptions = base_subscriptions.clone();
    attached_subscriptions.subscribe(matcher, matcher_output);
    compiler.set_output_subscriptions(attached_subscriptions);
    compiler
        .apply(&mut run, compiler.lowerer().lower(widget.graph()).unwrap())
        .unwrap();

    loop {
        let observed = lanes.opaque_lanes().iter().any(|lane| {
            lane.name().contains("Word Matcher.Match")
                && lane
                    .snapshot(CollectedLaneSnapshotRequest {
                        start_time_ns: 0,
                        end_time_ns: u64::MAX,
                        max_items: usize::MAX,
                    })
                    .and_then(|snapshot| snapshot.value::<TriggerLaneSnapshot>())
                    .is_some_and(|snapshot| {
                        matches!(snapshot.as_ref(), TriggerLaneSnapshot::Exact(markers) if !markers.is_empty())
                    })
        });
        if observed {
            break;
        }
        assert!(!run.is_finished());
        std::thread::sleep(Duration::from_millis(200));
    }
    widget.graph_mut().remove_node(matcher);
    compiler.set_output_subscriptions(base_subscriptions);
    compiler
        .apply(&mut run, compiler.lowerer().lower(widget.graph()).unwrap())
        .unwrap();
    run.wait();
    assert_outputs_equal(&actual, &expected);
}

fn print_usage() {
    eprintln!(
        "usage: cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
         <command> <capture.dsl>\n\
         \n\
         commands:\n\
           baseline               report timing, resources, storage, and output identity\n\
           waveform-index-profile profile cold waveform-index construction stages\n\
           waveform-index-persistent-profile\n\
                                  profile cold construction with native durable storage\n\
           waveform-index-concurrency-profile\n\
                                  sweep native durable builds across worker counts\n\
           derived-storage-profile\n\
                                  profile durable derived-cache generation by artifact class\n\
           compiler-runtime-memory time graph processing with derived artifacts in memory\n\
           protocol-selection     validate and compare indexed and packed parallel decoding\n\
           phase-one-runtime      time the phase-one reference pipeline\n\
           current-runtime        time the current reference pipeline\n\
           live-viewer-runtime    exercise live viewer queries while processing\n\
           validate-compiled      compare compiled and reference output\n\
           validate-live-attach   verify attach/detach preserves writer output\n\
           all                    run every command"
    );
}

fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(true)
        .try_init();
    let arguments = std::env::args_os()
        .skip(1)
        .filter(|argument| argument != "--bench")
        .collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "__reference")
    {
        assert_eq!(arguments.len(), 3, "invalid reference-stage arguments");
        run_current_reference(Path::new(&arguments[1]), Path::new(&arguments[2]));
        return;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "__compiled")
    {
        assert_eq!(arguments.len(), 4, "invalid compiled-stage arguments");
        run_compiled_validation_stage(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
            Path::new(&arguments[3]),
        );
        return;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "__baseline")
    {
        assert_eq!(arguments.len(), 5, "invalid baseline-stage arguments");
        run_compiled_baseline_stage(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
            Path::new(&arguments[3]),
            Path::new(&arguments[4]),
        );
        return;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "__cancel")
    {
        assert_eq!(arguments.len(), 5, "invalid cancellation-stage arguments");
        run_cancellation_stage(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
            Path::new(&arguments[3]),
            Path::new(&arguments[4]),
        );
        return;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "__protocol-indexed" || argument == "__protocol-packed")
    {
        assert_eq!(arguments.len(), 5, "invalid protocol-stage arguments");
        let strategy = if arguments[0] == "__protocol-indexed" {
            ParallelInputStrategy::Indexed
        } else {
            ParallelInputStrategy::PackedStream
        };
        run_protocol_selection_stage(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
            Path::new(&arguments[3]),
            Path::new(&arguments[4]),
            strategy,
        );
        return;
    }
    let mut arguments = arguments.into_iter();
    let Some(command) = arguments.next() else {
        print_usage();
        return;
    };
    if command == "--help" || command == "-h" {
        print_usage();
        return;
    }
    let Some(capture) = arguments.next().map(PathBuf::from) else {
        print_usage();
        panic!("a capture path is required");
    };
    assert!(arguments.next().is_none(), "unexpected extra arguments");
    assert!(
        capture.is_file(),
        "capture file '{}' does not exist",
        capture.display()
    );

    match command.to_string_lossy().as_ref() {
        "baseline" => reference_pipeline_baseline(&capture),
        "waveform-index-profile" => waveform_index_profile(&capture),
        "waveform-index-persistent-profile" => persistent_waveform_index_profile(&capture),
        "waveform-index-concurrency-profile" => waveform_index_concurrency_profile(&capture),
        "derived-storage-profile" => derived_storage_profile(&capture),
        "compiler-runtime-memory" => in_memory_compiler_runtime_benchmark(&capture),
        "protocol-selection" => protocol_selection_benchmark(&capture),
        "phase-one-runtime" => phase_one_reference_runtime_benchmark(&capture),
        "current-runtime" => current_reference_runtime_benchmark(&capture),
        "live-viewer-runtime" => live_viewer_subscription_benchmark(&capture),
        "validate-compiled" => compiled_graph_matches_current_reference(&capture),
        "validate-live-attach" => live_attach_detach_preserves_writer_output(&capture),
        "all" => {
            in_memory_compiler_runtime_benchmark(&capture);
            protocol_selection_benchmark(&capture);
            phase_one_reference_runtime_benchmark(&capture);
            current_reference_runtime_benchmark(&capture);
            live_viewer_subscription_benchmark(&capture);
            compiled_graph_matches_current_reference(&capture);
            live_attach_detach_preserves_writer_output(&capture);
        }
        command => {
            print_usage();
            panic!("unknown command '{command}'");
        }
    }
}
