//! Serializable workload, measurement, identity, baseline, and comparison records.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkloadSpec {
    pub(crate) schema_version: u32,
    pub(crate) name: String,
    pub(crate) graph: PathBuf,
    #[serde(default)]
    pub(crate) working_directory: Option<PathBuf>,
    #[serde(default)]
    pub(crate) capture: Option<CaptureSpec>,
    #[serde(default = "default_warmups")]
    pub(crate) warmup_runs: usize,
    #[serde(default = "default_measurements")]
    pub(crate) measured_runs: usize,
    #[serde(default = "default_acceptance_metrics")]
    pub(crate) acceptance_metrics: Vec<MetricName>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaptureSpec {
    pub(crate) graph_json_pointers: Vec<String>,
    #[serde(default)]
    pub(crate) path_environment: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetricName {
    WallSeconds,
    CpuSeconds,
    PeakRssBytes,
    ExecutionSeconds,
}

impl MetricName {
    pub(crate) const ALL: [MetricName; 4] = [
        MetricName::WallSeconds,
        MetricName::CpuSeconds,
        MetricName::PeakRssBytes,
        MetricName::ExecutionSeconds,
    ];
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct HeadlessReport {
    pub(crate) execution_seconds: f64,
    pub(crate) total_seconds: f64,
    pub(crate) capture_samples: Option<u64>,
    pub(crate) derived_lane_count: usize,
    pub(crate) derived_item_count: Option<u64>,
    pub(crate) derived_cache_bytes: u64,
    pub(crate) caches: Vec<HeadlessCacheReport>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct HeadlessCacheReport {
    pub(crate) node_id: u32,
    pub(crate) cache_key: String,
    pub(crate) data_fingerprint: String,
    pub(crate) total_bytes: u64,
    pub(crate) data_bytes: u64,
    pub(crate) index_bytes: u64,
    pub(crate) item_count: u64,
    pub(crate) block_count: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct OutputIdentity {
    pub(crate) capture_samples: Option<u64>,
    pub(crate) derived_lane_count: usize,
    pub(crate) derived_item_count: Option<u64>,
    pub(crate) derived_cache_bytes: u64,
    pub(crate) caches: Vec<HeadlessCacheReport>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct Measurement {
    pub(crate) wall_seconds: f64,
    pub(crate) cpu_seconds: f64,
    pub(crate) peak_rss_bytes: u64,
    pub(crate) execution_seconds: f64,
    pub(crate) reported_total_seconds: f64,
}

impl Measurement {
    pub(crate) fn metric(self, name: MetricName) -> f64 {
        match name {
            MetricName::WallSeconds => self.wall_seconds,
            MetricName::CpuSeconds => self.cpu_seconds,
            MetricName::PeakRssBytes => self.peak_rss_bytes as f64,
            MetricName::ExecutionSeconds => self.execution_seconds,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub(crate) struct MetricSummary {
    pub(crate) median: f64,
    pub(crate) minimum: f64,
    pub(crate) maximum: f64,
    pub(crate) spread: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct MeasurementSet {
    pub(crate) samples: Vec<Measurement>,
    pub(crate) metrics: BTreeMap<MetricName, MetricSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct CaptureIdentity {
    pub(crate) canonical_path: String,
    pub(crate) byte_length: u64,
    pub(crate) content_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct BaselineMetadata {
    pub(crate) created_unix_seconds: u64,
    pub(crate) git_commit: Option<String>,
    pub(crate) git_dirty: Option<bool>,
    pub(crate) host: String,
    pub(crate) operating_system: String,
    pub(crate) architecture: String,
    pub(crate) capture: Option<CaptureIdentity>,
    pub(crate) viewer_latency: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Baseline {
    pub(crate) schema_version: u32,
    pub(crate) workload: String,
    pub(crate) acceptance_metrics: Vec<MetricName>,
    pub(crate) metadata: BaselineMetadata,
    pub(crate) output_identity: OutputIdentity,
    pub(crate) measurements: MeasurementSet,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetricAssessment {
    Improved,
    Regressed,
    Overlapping,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricComparison {
    pub(crate) reference: MetricSummary,
    pub(crate) candidate: MetricSummary,
    pub(crate) median_change_percent: f64,
    pub(crate) assessment: MetricAssessment,
    pub(crate) acceptance_metric: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    Retain,
    Reject,
    Inconclusive,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ComparisonReport {
    pub(crate) schema_version: u32,
    pub(crate) workload: String,
    pub(crate) reference_source: String,
    pub(crate) identities_match: bool,
    pub(crate) reference: MeasurementSet,
    pub(crate) candidate: MeasurementSet,
    pub(crate) metrics: BTreeMap<MetricName, MetricComparison>,
    pub(crate) verdict: Verdict,
    pub(crate) viewer_latency: String,
}

pub(crate) fn output_identity(report: &HeadlessReport) -> OutputIdentity {
    let mut caches = report.caches.clone();
    caches.sort();
    OutputIdentity {
        capture_samples: report.capture_samples,
        derived_lane_count: report.derived_lane_count,
        derived_item_count: report.derived_item_count,
        derived_cache_bytes: report.derived_cache_bytes,
        caches,
    }
}

fn default_warmups() -> usize {
    1
}

fn default_measurements() -> usize {
    5
}

fn default_acceptance_metrics() -> Vec<MetricName> {
    vec![MetricName::WallSeconds]
}
