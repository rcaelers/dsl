//! Sigrok session (`.sr`) processing-node file source.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use signal_processing::{
    ArtifactRepository, CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory,
    InlineWorkExecutor, InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, Result,
    Sample, SampleBlock, SampleKind, Sender, SourceIdentity, WorkError, WorkExecutor, WorkResult,
    WorkTask,
};

use crate::support::capture_index::capture_cache_identity;
use crate::support::sigrok_file::{SigrokCapture, SigrokFileCaptureDataSource};

/// A PulseView/sigrok session source.
pub struct SigrokFileSource {
    name: String,
    capture: SigrokCapture,
    num_channels: usize,
    shutdown: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
    reader_tasks: Option<Vec<Box<dyn WorkTask>>>,
    spawned: bool,
    num_threads: usize,
    work_executor: Arc<dyn WorkExecutor>,
}

struct ChannelStream {
    samples: Arc<[u8]>,
    unitsize: usize,
    channel: usize,
    total_samples: usize,
    timestamp_step: u64,
    sender: Sender<Sample>,
    shutdown: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
}

struct ChannelBlockStream {
    samples: Arc<[u8]>,
    unitsize: usize,
    channel: usize,
    total_samples: usize,
    timestamp_step: u64,
    sender: Sender<SampleBlock>,
    shutdown: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
}

impl ChannelStream {
    fn run(self) {
        let value_at = |sample| {
            self.samples[sample * self.unitsize + self.channel / 8] & (1 << (self.channel % 8)) != 0
        };
        let mut current = value_at(0);
        if self.sender.send(Sample::new(current, 0)).is_ok() {
            for sample in 1..self.total_samples {
                if self.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                let value = value_at(sample);
                if value != current {
                    current = value;
                    if self
                        .sender
                        .send(Sample::new(value, sample as u64 * self.timestamp_step))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        self.sender.close();
        self.completed.fetch_add(1, Ordering::Relaxed);
    }
}

impl ChannelBlockStream {
    fn run(self) {
        let mut packed = vec![0_u8; self.total_samples.div_ceil(8)];
        for sample in 0..self.total_samples {
            if self.shutdown.load(Ordering::Relaxed) {
                break;
            }
            if self.samples[sample * self.unitsize + self.channel / 8] & (1 << (self.channel % 8))
                != 0
            {
                packed[sample / 8] |= 1 << (sample % 8);
            }
        }
        if !self.shutdown.load(Ordering::Relaxed) {
            let _ = self.sender.send(SampleBlock::new(
                packed,
                0,
                self.total_samples,
                self.timestamp_step,
            ));
        }
        self.sender.close();
        self.completed.fetch_add(1, Ordering::Relaxed);
    }
}

struct SigrokCaptureIndexFactory {
    path: PathBuf,
}

impl CaptureIndexFactory for SigrokCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.path.display().to_string()
    }

    fn open(
        self: Box<Self>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut dyn FnMut(CaptureIndexBuildProgress),
    ) -> Result<Box<dyn CaptureIndex + Send>> {
        let source = SigrokFileCaptureDataSource::open(&self.path)?;
        signal_processing::IndexSampler::open_data_source_with_executor_and_progress(
            source,
            artifact_repository,
            work_executor,
            |value| {
                progress(CaptureIndexBuildProgress {
                    completed: value.completed_roots,
                    total: value.total_roots,
                });
            },
        )
        .map(|index| Box::new(index) as Box<dyn CaptureIndex + Send>)
    }
}

impl SigrokFileSource {
    /// Creates the generic indexed-capture presentation for a static sigrok file.
    pub fn indexed_capture_presentation(
        path: impl AsRef<Path>,
    ) -> signal_processing::IndexedCapturePresentation {
        let path = path.as_ref().to_path_buf();
        signal_processing::IndexedCapturePresentation {
            identity: SourceIdentity::from_bytes(
                *blake3::hash(path.to_string_lossy().as_bytes()).as_bytes(),
            ),
            factory: Box::new(SigrokCaptureIndexFactory { path }),
        }
    }

    /// Returns the persistent-cache identity for a static sigrok file.
    pub fn capture_cache_identity(path: impl AsRef<Path>) -> Result<[u8; 32]> {
        let path = path.as_ref();
        let source = SigrokFileCaptureDataSource::open(path)?;
        Ok(capture_cache_identity(path, &source))
    }

    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let capture = SigrokCapture::open(path, 1)?;
        Ok(Self::from_capture(capture))
    }

    fn from_capture(capture: SigrokCapture) -> Self {
        let num_channels = capture.metadata().total_probes;
        Self {
            name: "sigrok_file_source".into(),
            capture,
            num_channels,
            shutdown: Arc::new(AtomicBool::new(false)),
            completed: Arc::new(AtomicUsize::new(0)),
            reader_tasks: None,
            spawned: false,
            num_threads: 0,
            work_executor: Arc::new(InlineWorkExecutor),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Selects the host executor used for independently scheduled readers.
    pub fn with_work_executor(mut self, work_executor: Arc<dyn WorkExecutor>) -> Self {
        self.work_executor = work_executor;
        self
    }

    pub fn header(&self) -> &signal_processing::CaptureMetadata {
        self.capture.metadata()
    }
}

impl ProcessNode for SigrokFileSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn should_stop(&self) -> bool {
        self.spawned && self.completed.load(Ordering::Relaxed) >= self.num_threads
    }
    fn is_self_threading(&self) -> bool {
        true
    }
    fn num_inputs(&self) -> usize {
        0
    }
    fn num_outputs(&self) -> usize {
        self.num_channels
    }
    fn output_schema(&self) -> Vec<PortSchema> {
        (0..self.num_channels)
            .map(|channel| {
                PortSchema::new::<Sample>(format!("ch{channel}"), channel, PortDirection::Output)
                    .with_sample_kinds(vec![SampleKind::Block, SampleKind::Edge])
            })
            .collect()
    }
    fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        if self.spawned {
            return Err(WorkError::NodeError(
                "work() called multiple times on sigrok file source".into(),
            ));
        }
        self.spawned = true;
        let timestamp_step = (1_000_000_000.0 / self.capture.metadata().samplerate_hz) as u64;
        let mut reader_tasks = Vec::new();
        for channel in 0..self.num_channels {
            let Some(output) = outputs.get(channel) else {
                continue;
            };
            if let Some(senders) = output.split_senders::<Sample>() {
                for sender in senders {
                    let samples = self.capture.samples();
                    let shutdown = Arc::clone(&self.shutdown);
                    let completed = Arc::clone(&self.completed);
                    let unitsize = self.capture.unitsize();
                    let total_samples = self.capture.metadata().total_samples as usize;
                    let task = self
                        .work_executor
                        .submit_long_running(Box::new(move || {
                            ChannelStream {
                                samples,
                                unitsize,
                                channel,
                                total_samples,
                                timestamp_step,
                                sender,
                                shutdown,
                                completed,
                            }
                            .run()
                        }))
                        .map_err(WorkError::NodeError)?;
                    reader_tasks.push(task);
                }
            }
            if let Some(senders) = output.split_senders::<SampleBlock>() {
                for sender in senders {
                    let samples = self.capture.samples();
                    let shutdown = Arc::clone(&self.shutdown);
                    let completed = Arc::clone(&self.completed);
                    let unitsize = self.capture.unitsize();
                    let total_samples = self.capture.metadata().total_samples as usize;
                    let task = self
                        .work_executor
                        .submit_long_running(Box::new(move || {
                            ChannelBlockStream {
                                samples,
                                unitsize,
                                channel,
                                total_samples,
                                timestamp_step,
                                sender,
                                shutdown,
                                completed,
                            }
                            .run()
                        }))
                        .map_err(WorkError::NodeError)?;
                    reader_tasks.push(task);
                }
            }
        }
        self.num_threads = reader_tasks.len();
        self.reader_tasks = Some(reader_tasks);
        Ok(0)
    }
}

impl Drop for SigrokFileSource {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(tasks) = self.reader_tasks.take() {
            for task in tasks {
                task.wait();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use signal_processing::capture::{CaptureDataSource, CaptureSource};
    use signal_processing::{
        CompletedWorkTask, OutputPort, Sender, Watchdog, WorkExecutor, WorkExecutorTask, WorkTask,
    };

    use super::*;
    use crate::support::capture_archive::CaptureArchive;
    use crate::support::sigrok_file::SigrokFileCaptureDataSource;

    fn fixture(version: &str, chunked: bool) -> SigrokCapture {
        let mut archive = TestCaptureArchive::default()
            .with_entry("version", version.as_bytes())
            .with_entry(
                "metadata",
                b"[device 1]\ncapturefile=logic-1\ntotal probes=8\nsamplerate=1 MHz\nprobe1=TX\nunitsize=1\n",
            )
            .with_entry(
                if chunked { "logic-1-1" } else { "logic-1" },
                &[0, 1, 1, 0, 0, 1, 0, 1],
            );
        SigrokCapture::from_archive(&mut archive, 1).unwrap()
    }

    struct RecordingReaderExecutor {
        submitted: AtomicUsize,
    }

    impl WorkExecutor for RecordingReaderExecutor {
        fn available_parallelism(&self) -> usize {
            1
        }

        fn submit(
            &self,
            _task: WorkExecutorTask,
        ) -> std::result::Result<Box<dyn WorkTask>, String> {
            Err("finite work must not schedule a file reader".into())
        }

        fn submit_long_running(
            &self,
            task: WorkExecutorTask,
        ) -> std::result::Result<Box<dyn WorkTask>, String> {
            self.submitted.fetch_add(1, Ordering::Relaxed);
            task();
            Ok(Box::new(CompletedWorkTask))
        }
    }

    #[test]
    fn source_uses_an_injected_version_two_capture() {
        let source = SigrokFileSource::from_capture(fixture("2", true));
        assert_eq!(source.header().total_probes, 8);
        assert_eq!(source.header().samplerate_hz, 1_000_000.0);
        assert_eq!(source.header().total_samples, 8);
        assert_eq!(source.header().probe_names[0], "TX");
        assert_eq!(source.num_outputs(), 8);
    }

    #[test]
    fn data_source_is_private_support_for_the_node() {
        let source =
            SigrokFileCaptureDataSource::from_capture("virtual/hello.sr", 123, fixture("2", true));
        assert_eq!(source.metadata().total_samples, 8);
        assert_eq!(source.open_reader().unwrap().metadata().total_probes, 8);
        assert_eq!(source.fingerprint().revision, 123);
    }

    #[test]
    fn opens_version_one_session_with_unchunked_logic_data() {
        let capture = fixture("1", false);
        let source = SigrokFileSource::from_capture(capture.clone());
        assert_eq!(source.header().total_probes, 8);
        assert_eq!(source.header().total_samples, 8);
        assert!(
            source
                .output_schema()
                .iter()
                .all(|port| { port.sample_kinds == [SampleKind::Block, SampleKind::Edge] })
        );

        let data_source =
            SigrokFileCaptureDataSource::from_capture("virtual/hello.sr", 123, capture);
        assert_eq!(
            data_source.open_reader().unwrap().metadata().total_samples,
            8
        );
    }

    #[test]
    fn source_schedules_readers_through_the_injected_executor() {
        let executor = Arc::new(RecordingReaderExecutor {
            submitted: AtomicUsize::new(0),
        });
        let mut source =
            SigrokFileSource::from_capture(fixture("2", true)).with_work_executor(executor.clone());
        let watchdog = Watchdog::new();
        let (sender, _receiver) = crossbeam_channel::bounded(16);
        let outputs = [OutputPort::new_with_watchdog(
            Sender::<Sample>::new(vec![sender]),
            &watchdog,
            "source",
            "ch0",
        )];

        source.work(&[], &outputs).unwrap();

        assert_eq!(executor.submitted.load(Ordering::Relaxed), 1);
        assert!(source.should_stop());
    }

    #[derive(Default)]
    struct TestCaptureArchive {
        entries: BTreeMap<String, Vec<u8>>,
    }

    impl TestCaptureArchive {
        fn with_entry(mut self, name: &str, data: &[u8]) -> Self {
            self.entries.insert(name.to_owned(), data.to_vec());
            self
        }
    }

    impl CaptureArchive for TestCaptureArchive {
        fn entry_names(&self) -> Vec<String> {
            self.entries.keys().cloned().collect()
        }

        fn entry_size(&mut self, name: &str) -> Result<Option<u64>> {
            Ok(self.entries.get(name).map(|entry| entry.len() as u64))
        }

        fn read_entry(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
            Ok(self.entries.get(name).cloned())
        }
    }
}
