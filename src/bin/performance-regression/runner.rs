//! Workload preparation, alternating execution, identity enforcement, and report persistence.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::time::{SystemTime, UNIX_EPOCH};

use tempfile::NamedTempFile;

use super::cli::{Arguments, Command, CommonArguments};
use super::comparison::{compare, summarize};
use super::model::{
    Baseline, BaselineMetadata, CaptureIdentity, HeadlessReport, Measurement, MeasurementSet,
    MetricName, OutputIdentity, SCHEMA_VERSION, Verdict, WorkloadSpec, output_identity,
};

type RunnerResult<T> = Result<T, Box<dyn std::error::Error>>;

struct PreparedWorkload {
    spec: WorkloadSpec,
    graph: NamedTempFile,
    working_directory: PathBuf,
    capture_identity: Option<CaptureIdentity>,
    warmups: usize,
    measurements: usize,
}

pub(crate) fn run(arguments: Arguments) -> RunnerResult<()> {
    match arguments.command {
        Command::Record(arguments) => {
            if arguments.baseline.exists() && !arguments.force {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "baseline already exists: {}; pass --force to replace it",
                        arguments.baseline.display()
                    ),
                )
                .into());
            }
            let prepared = prepare_workload(&arguments.common)?;
            let binary = canonical_file(&arguments.binary, "benchmark executable")?;
            let (measurements, identity) = measure_single(&prepared, &binary)?;
            let baseline = Baseline {
                schema_version: SCHEMA_VERSION,
                workload: prepared.spec.name.clone(),
                acceptance_metrics: prepared.spec.acceptance_metrics.clone(),
                metadata: baseline_metadata(&prepared),
                output_identity: identity,
                measurements,
            };
            write_json(&arguments.baseline, &baseline)?;
            println!(
                "recorded {} measured runs in {}",
                prepared.measurements,
                arguments.baseline.display()
            );
            Ok(())
        }
        Command::Compare(arguments) => {
            let prepared = prepare_workload(&arguments.common)?;
            let baseline = load_baseline(&arguments.baseline, &prepared)?;
            let candidate = canonical_file(&arguments.candidate, "candidate executable")?;
            let (reference, candidate, reference_source) =
                if let Some(reference) = arguments.reference {
                    let reference = canonical_file(&reference, "reference executable")?;
                    let (
                        reference_measurements,
                        reference_identity,
                        candidate_measurements,
                        candidate_identity,
                    ) = measure_pair(&prepared, &reference, &candidate)?;
                    require_identity("reference", &baseline.output_identity, &reference_identity)?;
                    require_identity("candidate", &baseline.output_identity, &candidate_identity)?;
                    (
                        reference_measurements,
                        candidate_measurements,
                        reference.display().to_string(),
                    )
                } else {
                    let (candidate_measurements, candidate_identity) =
                        measure_single(&prepared, &candidate)?;
                    require_identity("candidate", &baseline.output_identity, &candidate_identity)?;
                    (
                        baseline.measurements.clone(),
                        candidate_measurements,
                        arguments.baseline.display().to_string(),
                    )
                };
            let report = compare(
                prepared.spec.name.clone(),
                reference_source,
                reference,
                candidate,
                &baseline.acceptance_metrics,
            );
            if let Some(output) = &arguments.output {
                write_json(output, &report)?;
            }
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report.verdict == Verdict::Retain {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "comparison verdict is {:?}; candidate is not eligible for retention",
                    report.verdict
                ))
                .into())
            }
        }
    }
}

fn prepare_workload(arguments: &CommonArguments) -> RunnerResult<PreparedWorkload> {
    let workload_path = canonical_file(&arguments.workload, "workload specification")?;
    let base = workload_path
        .parent()
        .expect("canonical workload path has a parent");
    let mut spec: WorkloadSpec = serde_json::from_slice(&fs::read(&workload_path)?)?;
    validate_spec(&spec)?;
    let graph_path = resolve(base, &spec.graph);
    let mut graph: serde_json::Value = serde_json::from_slice(&fs::read(&graph_path)?)?;
    let capture_path = capture_path(arguments, &spec)?;
    let capture_identity = capture_path.as_deref().map(capture_identity).transpose()?;
    if let (Some(capture), Some(path)) = (&spec.capture, &capture_path) {
        let path = path.to_string_lossy().into_owned();
        for pointer in &capture.graph_json_pointers {
            let value = graph.pointer_mut(pointer).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("capture JSON pointer does not exist: {pointer}"),
                )
            })?;
            *value = serde_json::Value::String(path.clone());
        }
    }
    let mut temporary = NamedTempFile::new()?;
    serde_json::to_writer_pretty(&mut temporary, &graph)?;
    temporary.flush()?;
    let working_directory = spec
        .working_directory
        .as_ref()
        .map(|path| resolve(base, path))
        .unwrap_or_else(|| base.to_path_buf());
    let working_directory = working_directory.canonicalize()?;
    let warmups = arguments.warmups.unwrap_or(spec.warmup_runs);
    let measurements = arguments.runs.unwrap_or(spec.measured_runs);
    spec.graph = graph_path;
    Ok(PreparedWorkload {
        spec,
        graph: temporary,
        working_directory,
        capture_identity,
        warmups,
        measurements,
    })
}

fn validate_spec(spec: &WorkloadSpec) -> RunnerResult<()> {
    if spec.schema_version != SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unsupported workload schema version {}",
                spec.schema_version
            ),
        )
        .into());
    }
    if spec.name.trim().is_empty() || spec.warmup_runs == 0 || spec.measured_runs == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workload name and positive warmup/measured run counts are required",
        )
        .into());
    }
    if spec.acceptance_metrics.is_empty()
        || spec
            .acceptance_metrics
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != spec.acceptance_metrics.len()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "acceptance_metrics must be nonempty and unique",
        )
        .into());
    }
    if let Some(capture) = &spec.capture
        && capture.graph_json_pointers.is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture.graph_json_pointers must not be empty",
        )
        .into());
    }
    Ok(())
}

fn capture_path(arguments: &CommonArguments, spec: &WorkloadSpec) -> RunnerResult<Option<PathBuf>> {
    let configured = arguments.capture.clone().or_else(|| {
        spec.capture
            .as_ref()?
            .path_environment
            .as_ref()
            .and_then(std::env::var_os)
            .map(PathBuf::from)
    });
    match (&spec.capture, configured) {
        (Some(_), Some(path)) => Ok(Some(canonical_file(&path, "capture")?)),
        (Some(_), None) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workload requires --capture or its configured capture environment variable",
        )
        .into()),
        (None, Some(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--capture was supplied but the workload has no capture contract",
        )
        .into()),
        (None, None) => Ok(None),
    }
}

fn measure_single(
    workload: &PreparedWorkload,
    binary: &Path,
) -> RunnerResult<(MeasurementSet, OutputIdentity)> {
    for index in 0..workload.warmups {
        eprintln!("warmup {}/{}", index + 1, workload.warmups);
        execute_once(workload, binary)?;
    }
    let mut samples = Vec::with_capacity(workload.measurements);
    let mut identity = None;
    for index in 0..workload.measurements {
        eprintln!("measurement {}/{}", index + 1, workload.measurements);
        let (sample, observed) = execute_once(workload, binary)?;
        require_consistent_identity(&mut identity, observed)?;
        samples.push(sample);
    }
    Ok((
        summarize(samples),
        identity.expect("positive measurement count validated"),
    ))
}

fn measure_pair(
    workload: &PreparedWorkload,
    reference: &Path,
    candidate: &Path,
) -> RunnerResult<(
    MeasurementSet,
    OutputIdentity,
    MeasurementSet,
    OutputIdentity,
)> {
    for index in 0..workload.warmups {
        eprintln!("alternating warmup pair {}/{}", index + 1, workload.warmups);
        for (_, binary) in ordered_pair(index, reference, candidate) {
            execute_once(workload, binary)?;
        }
    }
    let mut reference_samples = Vec::with_capacity(workload.measurements);
    let mut candidate_samples = Vec::with_capacity(workload.measurements);
    let mut reference_identity = None;
    let mut candidate_identity = None;
    for index in 0..workload.measurements {
        eprintln!(
            "alternating measurement pair {}/{}",
            index + 1,
            workload.measurements
        );
        for (is_reference, binary) in ordered_pair(index, reference, candidate) {
            let (sample, identity) = execute_once(workload, binary)?;
            if is_reference {
                require_consistent_identity(&mut reference_identity, identity)?;
                reference_samples.push(sample);
            } else {
                require_consistent_identity(&mut candidate_identity, identity)?;
                candidate_samples.push(sample);
            }
        }
    }
    Ok((
        summarize(reference_samples),
        reference_identity.expect("positive measurement count validated"),
        summarize(candidate_samples),
        candidate_identity.expect("positive measurement count validated"),
    ))
}

fn ordered_pair<'a>(
    index: usize,
    reference: &'a Path,
    candidate: &'a Path,
) -> [(bool, &'a Path); 2] {
    if index.is_multiple_of(2) {
        [(true, reference), (false, candidate)]
    } else {
        [(false, candidate), (true, reference)]
    }
}

fn execute_once(
    workload: &PreparedWorkload,
    binary: &Path,
) -> RunnerResult<(Measurement, OutputIdentity)> {
    let output =
        super::process::execute(binary, workload.graph.path(), &workload.working_directory)?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{} failed with {}:\n{}",
            binary.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    let report: HeadlessReport = serde_json::from_slice(&output.stdout).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} emitted invalid JSON ({error}); stderr:\n{}",
                binary.display(),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    })?;
    Ok((
        Measurement {
            wall_seconds: output.wall_seconds,
            cpu_seconds: output.cpu_seconds,
            peak_rss_bytes: output.peak_rss_bytes,
            execution_seconds: report.execution_seconds,
            reported_total_seconds: report.total_seconds,
        },
        output_identity(&report),
    ))
}

fn require_consistent_identity(
    expected: &mut Option<OutputIdentity>,
    observed: OutputIdentity,
) -> RunnerResult<()> {
    if let Some(expected) = expected {
        require_identity("repeated run", expected, &observed)
    } else {
        *expected = Some(observed);
        Ok(())
    }
}

fn require_identity(
    label: &str,
    expected: &OutputIdentity,
    observed: &OutputIdentity,
) -> RunnerResult<()> {
    if expected == observed {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{label} output identity differs from the retained baseline\nexpected: {}\nobserved: {}",
            serde_json::to_string_pretty(expected)?,
            serde_json::to_string_pretty(observed)?,
        ))
        .into())
    }
}

fn load_baseline(path: &Path, workload: &PreparedWorkload) -> RunnerResult<Baseline> {
    let baseline: Baseline = serde_json::from_slice(&fs::read(path)?)?;
    if baseline.schema_version != SCHEMA_VERSION || baseline.workload != workload.spec.name {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "baseline schema or workload identity does not match",
        )
        .into());
    }
    if baseline.acceptance_metrics != workload.spec.acceptance_metrics
        || baseline.measurements.samples.is_empty()
        || MetricName::ALL
            .iter()
            .any(|metric| !baseline.measurements.metrics.contains_key(metric))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "baseline measurement contract does not match the workload",
        )
        .into());
    }
    if !capture_matches(
        baseline.metadata.capture.as_ref(),
        workload.capture_identity.as_ref(),
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "capture identity differs from the retained baseline",
        )
        .into());
    }
    Ok(baseline)
}

fn capture_matches(
    reference: Option<&CaptureIdentity>,
    candidate: Option<&CaptureIdentity>,
) -> bool {
    match (reference, candidate) {
        (Some(reference), Some(candidate)) => {
            reference.byte_length == candidate.byte_length
                && reference.content_fingerprint == candidate.content_fingerprint
        }
        (None, None) => true,
        _ => false,
    }
}

fn baseline_metadata(workload: &PreparedWorkload) -> BaselineMetadata {
    BaselineMetadata {
        created_unix_seconds: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        git_commit: command_text(&workload.working_directory, &["rev-parse", "HEAD"]),
        git_dirty: command_text(&workload.working_directory, &["status", "--porcelain"])
            .map(|status| !status.is_empty()),
        host: std::env::var("HOSTNAME")
            .ok()
            .or_else(|| hostname(&workload.working_directory))
            .unwrap_or_else(|| "unknown".to_owned()),
        operating_system: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        capture: workload.capture_identity.clone(),
        viewer_latency:
            "manual: record concurrent-viewer p50/p95/p99 and frames over 8 ms before retention"
                .to_owned(),
    }
}

fn command_text(directory: &Path, arguments: &[&str]) -> Option<String> {
    let output = ProcessCommand::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn hostname(directory: &Path) -> Option<String> {
    let output = ProcessCommand::new("hostname")
        .current_dir(directory)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn capture_identity(path: &Path) -> io::Result<CaptureIdentity> {
    let metadata = path.metadata()?;
    let mut reader = io::BufReader::new(fs::File::open(path)?);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(CaptureIdentity {
        canonical_path: path.display().to_string(),
        byte_length: metadata.len(),
        content_fingerprint: hasher.finalize().to_hex().to_string(),
    })
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> RunnerResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn canonical_file(path: &Path, label: &str) -> io::Result<PathBuf> {
    let path = path.canonicalize().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not resolve {label} '{}': {error}", path.display()),
        )
    })?;
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} is not a file: {}", path.display()),
        ));
    }
    Ok(path)
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(test)]
mod runner_tests {
    use super::*;
    use crate::model::MetricName;

    #[test]
    fn alternating_order_reverses_each_pair() {
        let reference = Path::new("reference");
        let candidate = Path::new("candidate");
        assert_eq!(
            ordered_pair(0, reference, candidate),
            [(true, reference), (false, candidate)]
        );
        assert_eq!(
            ordered_pair(1, reference, candidate),
            [(false, candidate), (true, reference)]
        );
    }

    #[test]
    fn workload_defaults_to_a_conservative_wall_time_decision() {
        let spec: WorkloadSpec =
            serde_json::from_str(r#"{"schema_version":1,"name":"demo","graph":"graph.json"}"#)
                .unwrap();
        assert_eq!(spec.warmup_runs, 1);
        assert_eq!(spec.measured_runs, 5);
        assert_eq!(spec.acceptance_metrics, [MetricName::WallSeconds]);
    }

    #[test]
    fn output_identity_is_order_independent_but_content_exact() {
        let cache = super::super::model::HeadlessCacheReport {
            node_id: 1,
            cache_key: "key".to_owned(),
            data_fingerprint: "fingerprint".to_owned(),
            total_bytes: 12,
            data_bytes: 8,
            index_bytes: 4,
            item_count: 3,
            block_count: 1,
        };
        let first = HeadlessReport {
            execution_seconds: 1.0,
            total_seconds: 1.0,
            capture_samples: Some(10),
            derived_lane_count: 2,
            derived_item_count: Some(6),
            derived_cache_bytes: 24,
            caches: vec![cache.clone(), cache.clone()],
        };
        let mut second = first.clone();
        second.caches.reverse();
        assert_eq!(output_identity(&first), output_identity(&second));
        second.caches[0].data_fingerprint = "different".to_owned();
        assert_ne!(output_identity(&first), output_identity(&second));
    }
}
