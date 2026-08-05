use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;

use logic_analyzer_graph_plan::ProcessingGraphError as CompileError;
use logic_analyzer_graph_runtime::{
    GraphRunContext, PreparedCapture, PreparedCaptureData, SourcePreparationStatus,
    SourcePreparationUpdate, SourceProcessOverrides,
};
use node_graph::{GraphState, NodeGraphWidget, NodeId};

use crate::app::supply_saved_timeline_cursors;
use crate::app_services::{AppServiceParts, AppServices};
use crate::graph_service::{GraphRun, GraphService};
use crate::viewer_selection::{output_subscription_plan, synchronize_viewer_compatibility};

const EXECUTION_POLL_INTERVAL: Duration = Duration::from_millis(2);
const PROGRESS_EVENT_INTERVAL: Duration = Duration::from_millis(250);

/// Progress emitted by the UI-equivalent headless graph runner.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum HeadlessRunEvent {
    Warning {
        message: String,
    },
    PreparingCapture {
        completed: u64,
        total: u64,
    },
    ClearingDerivedData,
    Running {
        elapsed_seconds: f64,
        capture_samples: Option<u64>,
        nodes: Vec<HeadlessNodeProgress>,
    },
}

/// Final processed-item count for one graph node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeadlessNodeProgress {
    pub node_id: u32,
    pub title: String,
    pub items: u64,
}

/// One persistent derived-data entry produced by a headless run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HeadlessCacheReport {
    pub node_id: u32,
    pub node_title: String,
    pub cache_key: String,
    pub total_bytes: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub item_count: u64,
    pub block_count: u64,
}

/// Timing, throughput, and durable-output summary for one complete run.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeadlessRunReport {
    pub graph_load_seconds: f64,
    pub source_preparation_seconds: f64,
    pub cache_clear_seconds: f64,
    pub execution_seconds: f64,
    pub total_seconds: f64,
    pub capture_samples: Option<u64>,
    pub capture_seconds: Option<f64>,
    pub realtime_factor: Option<f64>,
    pub cleared_cache_entries: usize,
    pub cleared_cache_bytes: u64,
    pub derived_lane_count: usize,
    pub derived_item_count: Option<u64>,
    pub derived_cache_bytes: u64,
    pub nodes: Vec<HeadlessNodeProgress>,
    pub caches: Vec<HeadlessCacheReport>,
    pub warnings: Vec<String>,
}

/// User-facing failure returned by headless graph loading or execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadlessRunError {
    message: String,
}

impl HeadlessRunError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HeadlessRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for HeadlessRunError {}

/// Executes saved graph documents with the same application and native-host
/// contracts used by the interactive Run command.
pub struct HeadlessGraphRunner {
    graph_service: Box<dyn GraphService>,
    host_service: Box<dyn crate::HostService>,
    work_executor: std::sync::Arc<dyn signal_runtime::WorkExecutor>,
}

impl HeadlessGraphRunner {
    /// Creates a headless runner from the same injected services used by the UI.
    ///
    /// # Parameters
    /// - `services`: Graph, host, repository, and executor services selected by composition.
    pub fn new(services: AppServices) -> Self {
        let AppServiceParts {
            graph_service,
            host_service,
            work_executor,
            ..
        } = services.into_parts();
        Self {
            graph_service,
            host_service,
            work_executor,
        }
    }

    /// Loads and executes a graph using the host's ordinary document loader.
    ///
    /// # Parameters
    /// - `path`: Graph-document path passed to the host loader.
    /// - `emit`: Callback receiving warnings and periodic execution progress.
    pub fn run_file(
        &mut self,
        path: &Path,
        emit: &mut dyn FnMut(HeadlessRunEvent),
    ) -> Result<HeadlessRunReport, HeadlessRunError> {
        let total_started = Instant::now();
        let load_started = Instant::now();
        let graph = self
            .host_service
            .load_graph(path)
            .map_err(HeadlessRunError::new)?;
        let graph = restore_graph(graph);
        let graph_load_seconds = load_started.elapsed().as_secs_f64();
        self.run_restored_graph(graph, graph_load_seconds, total_started, emit)
    }

    /// Executes an already-deserialized graph through the same restoration and
    /// application execution policy as [`Self::run_file`].
    ///
    /// # Parameters
    /// - `graph`: Persisted graph document to restore, migrate, and run.
    /// - `emit`: Callback receiving warnings and periodic execution progress.
    pub fn run_graph(
        &mut self,
        graph: GraphState,
        emit: &mut dyn FnMut(HeadlessRunEvent),
    ) -> Result<HeadlessRunReport, HeadlessRunError> {
        let total_started = Instant::now();
        let load_started = Instant::now();
        let graph = restore_graph(graph);
        let graph_load_seconds = load_started.elapsed().as_secs_f64();
        self.run_restored_graph(graph, graph_load_seconds, total_started, emit)
    }

    fn run_restored_graph(
        &mut self,
        mut graph: GraphState,
        graph_load_seconds: f64,
        total_started: Instant,
        emit: &mut dyn FnMut(HeadlessRunEvent),
    ) -> Result<HeadlessRunReport, HeadlessRunError> {
        let mut warnings = synchronize_viewer_compatibility(&mut graph)
            .map_err(|error| HeadlessRunError::new(error.to_string()))?
            .into_iter()
            .map(|warning| warning.message)
            .collect::<Vec<_>>();
        for warning in &warnings {
            emit(HeadlessRunEvent::Warning {
                message: warning.clone(),
            });
        }
        self.graph_service
            .set_output_subscriptions(output_subscription_plan(&graph));

        let preparation_started = Instant::now();
        let prepared = self.prepare_capture(&graph, emit)?;
        let source_preparation_seconds = preparation_started.elapsed().as_secs_f64();
        let (capture_samples, capture_seconds) = capture_extent(prepared.as_ref());

        emit(HeadlessRunEvent::ClearingDerivedData);
        let cache_clear_started = Instant::now();
        let cache_configs = self.cache_configs(&graph)?;
        let (cleared_cache_entries, cleared_cache_bytes) =
            self.clear_caches(cache_configs.values().map(|(_, config)| config))?;
        let cache_clear_seconds = cache_clear_started.elapsed().as_secs_f64();

        let mut context = GraphRunContext::default();
        supply_saved_timeline_cursors(&graph, &mut context).map_err(HeadlessRunError::new)?;
        let lanes = context.derived_lanes().clone();
        let execution_started = Instant::now();
        let mut run = self
            .graph_service
            .start_run(&graph, &mut context, SourceProcessOverrides::new())
            .map_err(|errors| compile_error(&graph, &errors))?;
        let mut last_progress_event = execution_started
            .checked_sub(PROGRESS_EVENT_INTERVAL)
            .unwrap_or(execution_started);
        while !run.is_finished() {
            run.pump_for(256, Duration::from_millis(4));
            if last_progress_event.elapsed() >= PROGRESS_EVENT_INTERVAL {
                emit(HeadlessRunEvent::Running {
                    elapsed_seconds: execution_started.elapsed().as_secs_f64(),
                    capture_samples,
                    nodes: node_progress(&graph, run.as_ref()),
                });
                last_progress_event = Instant::now();
            }
            self.work_executor.idle(EXECUTION_POLL_INTERVAL);
        }
        let nodes = node_progress(&graph, run.as_ref());
        run.wait();
        if let Some(failure) = run.take_failure() {
            return Err(HeadlessRunError::new(failure));
        }
        let node_failures = run.take_node_failures();
        if !node_failures.is_empty() {
            return Err(HeadlessRunError::new(
                node_failures
                    .into_iter()
                    .map(|(node, failure)| {
                        let owner = node
                            .and_then(|node| graph.nodes.get(&node))
                            .map(|node| node.title.as_str())
                            .unwrap_or(failure.node.as_str());
                        format!("{owner}: {}", failure.message)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
        for (node, disconnected) in run.take_disconnected() {
            let owner = node
                .and_then(|node| graph.nodes.get(&node))
                .map(|node| node.title.as_str())
                .unwrap_or("processing graph");
            warnings.push(format!(
                "{owner}: disconnected {}.{} from {}",
                disconnected.producer,
                disconnected.port,
                disconnected.consumer.as_deref().unwrap_or("a consumer")
            ));
        }
        let execution_seconds = execution_started.elapsed().as_secs_f64();
        let caches = self.cache_reports(&graph, &cache_configs)?;
        let derived_cache_bytes = caches.iter().map(|cache| cache.total_bytes).sum();
        let opaque_lanes = lanes.opaque_lanes();
        let derived_lane_count = opaque_lanes.len();
        let derived_item_count = opaque_lanes.iter().try_fold(0_u64, |total, lane| {
            lane.storage_snapshot()
                .retained_items
                .or_else(|| lane.table_metadata().map(|metadata| metadata.total_rows))
                .map(|items| total.saturating_add(items))
        });
        let realtime_factor = capture_seconds
            .filter(|_| execution_seconds > 0.0)
            .map(|duration| duration / execution_seconds);
        Ok(HeadlessRunReport {
            graph_load_seconds,
            source_preparation_seconds,
            cache_clear_seconds,
            execution_seconds,
            total_seconds: total_started.elapsed().as_secs_f64(),
            capture_samples,
            capture_seconds,
            realtime_factor,
            cleared_cache_entries,
            cleared_cache_bytes,
            derived_lane_count,
            derived_item_count,
            derived_cache_bytes,
            nodes,
            caches,
            warnings,
        })
    }

    fn prepare_capture(
        &mut self,
        graph: &GraphState,
        emit: &mut dyn FnMut(HeadlessRunEvent),
    ) -> Result<Option<PreparedCapture>, HeadlessRunError> {
        let mut last_progress = None;
        loop {
            match self.graph_service.synchronize_prepared_capture(graph) {
                SourcePreparationUpdate::Ready(prepared) => return Ok(Some(prepared)),
                SourcePreparationUpdate::Failed(error) => {
                    return Err(HeadlessRunError::new(format!(
                        "could not prepare capture source: {error}"
                    )));
                }
                SourcePreparationUpdate::Preparing(preparing) => {
                    if let Some(progress) = preparing.progress
                        && last_progress != Some(progress)
                    {
                        last_progress = Some(progress);
                        emit(HeadlessRunEvent::PreparingCapture {
                            completed: progress.completed,
                            total: progress.total,
                        });
                    }
                }
                SourcePreparationUpdate::Cleared => return Ok(None),
                SourcePreparationUpdate::Unchanged => {
                    match self.graph_service.source_preparation_status() {
                        SourcePreparationStatus::Empty => return Ok(None),
                        SourcePreparationStatus::Ready => return Ok(None),
                        SourcePreparationStatus::Failed(error) => {
                            return Err(HeadlessRunError::new(format!(
                                "could not prepare capture source: {error}"
                            )));
                        }
                        SourcePreparationStatus::Preparing => {}
                    }
                }
            }
            self.work_executor.idle(EXECUTION_POLL_INTERVAL);
        }
    }

    fn cache_configs(&self, graph: &GraphState) -> Result<CacheConfigs, HeadlessRunError> {
        let inventory = self
            .graph_service
            .derived_cache_configs_by_node(graph)
            .map_err(|errors| compile_error(graph, &errors))?;
        let mut unique = BTreeMap::new();
        for (node, configs) in inventory {
            for config in configs {
                unique.entry(config.cache_key).or_insert((node, config));
            }
        }
        Ok(unique)
    }

    fn clear_caches<'a>(
        &self,
        configs: impl Iterator<Item = &'a signal_derived::PersistentStoreConfig>,
    ) -> Result<(usize, u64), HeadlessRunError> {
        let mut entries = 0;
        let mut bytes = 0_u64;
        for config in configs {
            let cleared = self
                .graph_service
                .clear_derived_cache_entry(config)
                .map_err(HeadlessRunError::new)?;
            entries += cleared.removed_entries;
            bytes = bytes.saturating_add(cleared.removed_bytes);
        }
        Ok((entries, bytes))
    }

    fn cache_reports(
        &self,
        graph: &GraphState,
        configs: &CacheConfigs,
    ) -> Result<Vec<HeadlessCacheReport>, HeadlessRunError> {
        configs
            .iter()
            .filter_map(|(cache_key, (node, config))| {
                self.graph_service
                    .inspect_derived_cache_entry(config)
                    .transpose()
                    .map(|result| result.map(|snapshot| (cache_key, node, snapshot)))
            })
            .map(|result| {
                let (cache_key, node, snapshot) = result.map_err(HeadlessRunError::new)?;
                Ok(HeadlessCacheReport {
                    node_id: node.0,
                    node_title: graph
                        .nodes
                        .get(node)
                        .map(|node| node.title.clone())
                        .unwrap_or_else(|| "Removed node".to_owned()),
                    cache_key: hex(cache_key),
                    total_bytes: snapshot.total_bytes,
                    data_bytes: snapshot.data_bytes,
                    index_bytes: snapshot.index_bytes,
                    item_count: snapshot.item_count,
                    block_count: snapshot.index_item_count,
                })
            })
            .collect()
    }
}

type CacheConfigs = BTreeMap<[u8; 32], (NodeId, signal_derived::PersistentStoreConfig)>;

fn restore_graph(graph: GraphState) -> GraphState {
    let mut widget = NodeGraphWidget::new(crate::build_node_registry());
    widget.set_graph(graph);
    widget.graph().clone()
}

fn capture_extent(prepared: Option<&PreparedCapture>) -> (Option<u64>, Option<f64>) {
    match prepared.map(|prepared| &prepared.data) {
        Some(PreparedCaptureData::Indexed(index)) => {
            let metadata = index.current_metadata();
            (
                Some(metadata.total_samples),
                Some(metadata.duration_us() / 1_000_000.0),
            )
        }
        Some(PreparedCaptureData::InMemory { duration_us, .. }) => {
            (None, Some(*duration_us / 1_000_000.0))
        }
        Some(PreparedCaptureData::Channels(_)) | None => (None, None),
    }
}

fn node_progress(graph: &GraphState, run: &dyn GraphRun) -> Vec<HeadlessNodeProgress> {
    let mut progress = run
        .progress()
        .into_iter()
        .map(|(node, items)| HeadlessNodeProgress {
            node_id: node.0,
            title: graph
                .nodes
                .get(&node)
                .map(|node| node.title.clone())
                .unwrap_or_else(|| "Compiler-owned collector".to_owned()),
            items,
        })
        .collect::<Vec<_>>();
    progress.sort_by_key(|node| node.node_id);
    progress
}

fn compile_error(graph: &GraphState, errors: &[CompileError]) -> HeadlessRunError {
    HeadlessRunError::new(
        errors
            .iter()
            .map(|error| {
                let owner = error
                    .node
                    .and_then(|node| graph.nodes.get(&node))
                    .map(|node| node.title.as_str())
                    .unwrap_or("graph");
                format!("{owner}: {}", error.message)
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    result
}
