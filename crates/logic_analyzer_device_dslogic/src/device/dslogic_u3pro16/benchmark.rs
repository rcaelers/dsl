//! Developer benchmark owned by the U3Pro16 source implementation.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use logic_analyzer_acquisition::{CaptureMode, LogicCaptureConfig};
use platform_artifacts::MemoryArtifactRepository;
use platform_runtime::{WorkExecutor, WorkExecutorTask, WorkTask};
use signal_capture::CaptureIndex;
use signal_capture_session::{
    AcquisitionContext, CaptureCursorItem, CaptureSessionId, CaptureStore, CaptureStoreConfig,
    CaptureStoreCursor, CaptureStoreDescriptor, GrowingCaptureIndex, bounded_capture_event_queue,
};

use super::driver::DsLogicU3Pro16;
use super::streaming::StreamingProvider;
use super::transport::{LinkSpeed, UsbError, UsbTransport};

struct GeneratedStreamingTransport {
    control_reads: VecDeque<Vec<u8>>,
    header_pending: bool,
    data_bytes: usize,
    data_offset: usize,
}

struct BenchmarkWorkExecutor;

struct BenchmarkWorkTask {
    handle: Option<JoinHandle<()>>,
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

impl WorkExecutor for BenchmarkWorkExecutor {
    fn available_parallelism(&self) -> usize {
        1
    }

    fn submit(
        &self,
        task: WorkExecutorTask,
    ) -> Result<Box<dyn WorkTask>, platform_runtime::WorkExecutorError> {
        self.submit_long_running(task)
    }

    fn submit_long_running(
        &self,
        task: WorkExecutorTask,
    ) -> Result<Box<dyn WorkTask>, platform_runtime::WorkExecutorError> {
        Ok(Box::new(BenchmarkWorkTask {
            handle: Some(std::thread::spawn(task)),
        }))
    }
}

impl GeneratedStreamingTransport {
    fn new(data_bytes: usize) -> Self {
        Self {
            control_reads: VecDeque::from([
                vec![0x40],
                vec![2, 0],
                vec![0, 0, 0, 0, 0x0e],
                vec![0, 0, 0, 0, 0x0e],
                vec![2, 0],
                vec![0x08],
                vec![0x80],
            ]),
            header_pending: true,
            data_bytes,
            data_offset: 0,
        }
    }
}

impl UsbTransport for GeneratedStreamingTransport {
    fn link_speed(&self) -> LinkSpeed {
        LinkSpeed::Super
    }

    fn control_write(
        &mut self,
        _request_type: u8,
        _request: u8,
        _value: u16,
        _index: u16,
        data: &[u8],
        _timeout: Duration,
    ) -> Result<usize, UsbError> {
        Ok(data.len())
    }

    fn control_read(
        &mut self,
        _request_type: u8,
        _request: u8,
        _value: u16,
        _index: u16,
        data: &mut [u8],
        _timeout: Duration,
    ) -> Result<usize, UsbError> {
        let response = self.control_reads.pop_front().ok_or(UsbError::Other)?;
        if response.len() != data.len() {
            return Err(UsbError::Other);
        }
        data.copy_from_slice(&response);
        Ok(data.len())
    }

    fn bulk_write(
        &mut self,
        _endpoint: u8,
        data: &[u8],
        _timeout: Duration,
    ) -> Result<usize, UsbError> {
        Ok(data.len())
    }

    fn bulk_read(
        &mut self,
        _endpoint: u8,
        data: &mut [u8],
        _timeout: Duration,
    ) -> Result<usize, UsbError> {
        if self.header_pending {
            if data.len() != 1024 {
                return Err(UsbError::Other);
            }
            data.fill(0);
            data[..4].copy_from_slice(&0x5555_5555_u32.to_le_bytes());
            self.header_pending = false;
            return Ok(data.len());
        }
        let read = data.len().min(self.data_bytes - self.data_offset);
        data[..read].fill(0xa5);
        self.data_offset += read;
        Ok(read)
    }
}

/// Runs the generated U3Pro16 sustained-ingest benchmark.
///
/// This contract is available only through the opt-in `developer-tools`
/// feature. The generated USB transport and device internals remain private to
/// the source owner.
pub fn run_streaming_benchmark() {
    for (channels_count, rate_hz, samples) in [
        (3_usize, 1_000_000_000_u64, 32_000_000_u64),
        (16, 125_000_000, 32_000_000),
    ] {
        run_scenario(channels_count, rate_hz, samples);
    }
}

fn run_scenario(channels_count: usize, rate_hz: u64, samples: u64) {
    let input_mask = (1_u64 << channels_count) - 1;
    let mut config = LogicCaptureConfig::finite(rate_hz, input_mask, samples);
    config.mode = CaptureMode::Streaming;
    let data_bytes = usize::try_from(
        u128::from(samples)
            .checked_mul(channels_count as u128)
            .unwrap()
            .div_ceil(8),
    )
    .unwrap();
    let analyzer = DsLogicU3Pro16::new(GeneratedStreamingTransport::new(data_bytes)).unwrap();
    let channels = (0..channels_count)
        .map(|channel| signal_capture::CaptureChannelId::new(format!("u3pro16:input:{channel}")))
        .collect::<Vec<_>>();
    let provider = StreamingProvider::new(analyzer, config, channels.clone()).unwrap();
    let session_id = CaptureSessionId::new(0x9000 + channels_count as u128);
    let descriptor = CaptureStoreDescriptor::new(session_id, channels.clone()).unwrap();
    let (store, writer) = CaptureStore::create(CaptureStoreConfig::new(
        Arc::new(MemoryArtifactRepository::new()),
        descriptor,
    ))
    .unwrap();
    let (index, index_worker) = GrowingCaptureIndex::spawn(
        store.clone(),
        "U3 streaming benchmark",
        rate_hz as f64,
        (0..channels_count)
            .map(|channel| format!("Ch {channel}"))
            .collect(),
        Arc::new(BenchmarkWorkExecutor),
    )
    .unwrap();
    let viewer_stop = Arc::new(AtomicBool::new(false));
    let viewer_stop_worker = Arc::clone(&viewer_stop);
    let mut viewer_index = index.clone();
    let viewer_channels = (0..channels_count).collect::<Vec<_>>();
    let viewer = std::thread::spawn(move || {
        while !viewer_stop_worker.load(Ordering::Relaxed) {
            let total_samples = viewer_index.current_metadata().total_samples;
            if total_samples > 0 {
                let _ = viewer_index.sampled_window(&viewer_channels, 0, total_samples, 1_920);
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    });
    let analyzed_samples = Arc::new(AtomicU64::new(0));
    let analyzed_samples_worker = Arc::clone(&analyzed_samples);
    let mut slow_cursor = store.open_cursor().unwrap();
    let slow_consumer = std::thread::spawn(move || {
        loop {
            match slow_cursor.wait_next(Duration::from_millis(50)).unwrap() {
                CaptureCursorItem::Chunk(chunk) => {
                    analyzed_samples_worker.store(chunk.end_sample(), Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(1));
                }
                CaptureCursorItem::Pending => {}
                CaptureCursorItem::End => break,
            }
        }
    });
    let (events, _event_reader) = bounded_capture_event_queue(4096).unwrap();
    let context = AcquisitionContext::new(session_id, Box::new(writer), Box::new(events))
        .with_work_executor(Arc::new(BenchmarkWorkExecutor));
    let mut acquisition = provider.prepare(context).unwrap();

    let started = Instant::now();
    acquisition.start().unwrap();
    let outcome = acquisition.join().unwrap();
    let acquisition_elapsed = started.elapsed();
    let lag_at_finish = samples.saturating_sub(analyzed_samples.load(Ordering::Relaxed));
    let summary_lag_at_finish = samples.saturating_sub(index.current_metadata().total_samples);
    viewer_stop.store(true, Ordering::Relaxed);
    viewer.join().unwrap();
    let summary_started = Instant::now();
    index_worker.join().unwrap();
    slow_consumer.join().unwrap();
    let catch_up_elapsed = summary_started.elapsed();
    store.finalize().unwrap();

    let mib = data_bytes as f64 / (1024.0 * 1024.0);
    println!(
        "u3-stream channels={channels_count} rate_hz={rate_hz} samples={samples} data_mib={mib:.1} acquisition_s={:.3} ingest_mib_s={:.1} optional_consumer_lag_samples={lag_at_finish} summary_lag_samples={summary_lag_at_finish} summary_catchup_s={:.3} resident_summary_records={}",
        acquisition_elapsed.as_secs_f64(),
        mib / acquisition_elapsed.as_secs_f64(),
        catch_up_elapsed.as_secs_f64(),
        index.resident_summary_records(),
    );
    assert_eq!(outcome.captured_samples, samples);
    assert_eq!(store.snapshot().resident_commit_records, 0);
    assert!(index.resident_summary_records() <= channels_count * 64 * 12);
}
