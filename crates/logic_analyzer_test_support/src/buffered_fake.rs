//! Deterministic device-buffered provider used to challenge portable capture contracts.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use signal_capture_session::{
    AcquisitionContext, AcquisitionError, AcquisitionOutcome, AcquisitionResult,
    CaptureAcquisitionPhase, CaptureBufferPool, CaptureChannelId, CaptureChunk, CaptureCompletion,
    CaptureDataDelivery, CaptureProgress, CaptureProviderCapabilities, CaptureSessionId,
    CaptureSessionState, CaptureSettingCombination, PreparedAcquisition, SimpleTriggerCondition,
};

/// Configuration for a deterministic provider that captures on-device then uploads in chunks.
#[derive(Clone, Debug)]
pub struct BufferedFakeConfig {
    channels: Arc<[CaptureChannelId]>,
    sample_rate_hz: u64,
    total_samples: u64,
    upload_chunk_samples: u64,
    trigger_conditions: Arc<[Option<SimpleTriggerCondition>]>,
    seed: u64,
    capabilities: CaptureProviderCapabilities,
}

impl BufferedFakeConfig {
    /// Validates configuration for a deterministic buffered capture.
    ///
    /// # Parameters
    /// - `channels`: Non-empty channel set exposed by the fake.
    /// - `sample_rate_hz`: Non-zero acquisition sample rate.
    /// - `total_samples`: Non-zero number of samples captured before upload.
    /// - `upload_chunk_samples`: Non-zero maximum number of samples per uploaded chunk.
    /// - `seed`: Stable seed selecting the generated waveform.
    pub fn new(
        channels: impl Into<Arc<[CaptureChannelId]>>,
        sample_rate_hz: u64,
        total_samples: u64,
        upload_chunk_samples: u64,
        seed: u64,
    ) -> AcquisitionResult<Self> {
        let channels = channels.into();
        if channels.is_empty() {
            return Err(AcquisitionError::InvalidRequest(
                "buffered fake capture requires at least one channel".into(),
            ));
        }
        if sample_rate_hz == 0 || total_samples == 0 || upload_chunk_samples == 0 {
            return Err(AcquisitionError::InvalidRequest(
                "buffered fake rate, sample count, and upload chunk size must be non-zero".into(),
            ));
        }
        let mut setting_matrix = vec![
            CaptureSettingCombination::new(Arc::clone(&channels), Arc::from([sample_rate_hz]))
                .map_err(AcquisitionError::InvalidRequest)?,
        ];
        if channels.len() > 1 {
            let bank_subset = channels.iter().step_by(2).cloned().collect::<Vec<_>>();
            let faster_rate = sample_rate_hz.checked_mul(4).ok_or_else(|| {
                AcquisitionError::InvalidRequest("buffered fake sample rate overflows u64".into())
            })?;
            setting_matrix.push(
                CaptureSettingCombination::new(bank_subset, Arc::from([faster_rate]))
                    .map_err(AcquisitionError::InvalidRequest)?,
            );
        }
        let capabilities = CaptureProviderCapabilities::new(
            CaptureDataDelivery::BufferedUpload,
            setting_matrix,
            false,
        )
        .map_err(AcquisitionError::InvalidRequest)?;
        let config = Self {
            trigger_conditions: vec![None; channels.len()].into(),
            channels,
            sample_rate_hz,
            total_samples,
            upload_chunk_samples,
            seed,
            capabilities,
        };
        config.maximum_chunk_bytes()?;
        Ok(config)
    }

    /// Returns this value configured with simple trigger.
    ///
    /// # Parameters
    /// - `conditions`: One optional condition per configured channel; `None` and `Ignore` do not
    ///   constrain triggering.
    pub fn with_simple_trigger(
        mut self,
        conditions: impl Into<Arc<[Option<SimpleTriggerCondition>]>>,
    ) -> AcquisitionResult<Self> {
        let conditions = conditions.into();
        if conditions.len() != self.channels.len() {
            return Err(AcquisitionError::InvalidRequest(format!(
                "buffered fake trigger has {} channels, expected {}",
                conditions.len(),
                self.channels.len()
            )));
        }
        self.trigger_conditions = conditions;
        Ok(self)
    }

    /// Returns the configured capture channels in packed-data order.
    pub fn channels(&self) -> &[CaptureChannelId] {
        &self.channels
    }

    /// Returns the configured acquisition rate in hertz.
    pub const fn sample_rate_hz(&self) -> u64 {
        self.sample_rate_hz
    }

    /// Returns the finite number of captured samples before upload starts.
    pub const fn total_samples(&self) -> u64 {
        self.total_samples
    }

    /// Returns the supported channel/rate combinations and buffered-delivery behavior.
    pub fn capabilities(&self) -> &CaptureProviderCapabilities {
        &self.capabilities
    }

    /// Returns the deterministic level generated at a sample and channel index.
    ///
    /// # Parameters
    /// - `sample`: Absolute sample index within the finite capture.
    /// - `channel`: Zero-based index into [`Self::channels`].
    pub fn level_at(&self, sample: u64, channel: usize) -> bool {
        let channel = channel as u64;
        let period = channel.wrapping_mul(2).wrapping_add(3);
        ((sample / period) ^ channel ^ self.seed) & 1 != 0
    }

    /// Returns the first sample satisfying the configured trigger, if any.
    pub fn first_trigger_sample(&self) -> Option<u64> {
        if !self.has_trigger() {
            return None;
        }
        (0..self.total_samples).find(|sample| {
            self.trigger_conditions
                .iter()
                .enumerate()
                .all(|(channel, condition)| {
                    let Some(condition) = condition else {
                        return true;
                    };
                    let previous = sample
                        .checked_sub(1)
                        .map(|previous| self.level_at(previous, channel));
                    condition.matches(previous, self.level_at(*sample, channel))
                })
        })
    }

    fn has_trigger(&self) -> bool {
        self.trigger_conditions
            .iter()
            .flatten()
            .any(|condition| *condition != SimpleTriggerCondition::Ignore)
    }

    fn maximum_chunk_bytes(&self) -> AcquisitionResult<usize> {
        let samples = self.upload_chunk_samples.min(self.total_samples) as u128;
        let bits = samples
            .checked_mul(self.channels.len() as u128)
            .and_then(|bits| bits.checked_add(7))
            .ok_or_else(|| AcquisitionError::Internal("buffered fake payload overflow".into()))?;
        usize::try_from(bits.div_ceil(8))
            .map_err(|_| AcquisitionError::Internal("buffered fake payload is too large".into()))
    }

    fn build_chunk(
        &self,
        buffer_pool: &CaptureBufferPool,
        session_id: CaptureSessionId,
        sequence: u64,
        start_sample: u64,
        sample_count: u64,
    ) -> AcquisitionResult<CaptureChunk> {
        let bit_offset = ((sequence * 5 + 2) % 8) as u8;
        let bit_count = (sample_count as u128)
            .checked_mul(self.channels.len() as u128)
            .ok_or_else(|| AcquisitionError::Internal("buffered fake payload overflow".into()))?;
        let byte_count = usize::try_from((bit_count + u128::from(bit_offset)).div_ceil(8))
            .map_err(|_| AcquisitionError::Internal("buffered fake payload is too large".into()))?;
        let mut bytes = buffer_pool.acquire();
        bytes.resize(byte_count, 0);
        for relative_sample in 0..sample_count {
            for channel in 0..self.channels.len() {
                if self.level_at(start_sample + relative_sample, channel) {
                    let bit = usize::from(bit_offset)
                        + relative_sample as usize * self.channels.len()
                        + channel;
                    bytes.as_mut_slice()[bit / 8] |= 1 << (bit % 8);
                }
            }
        }
        CaptureChunk::packed_lsb_first(
            session_id,
            sequence,
            start_sample,
            sample_count,
            Arc::clone(&self.channels),
            bytes.freeze(),
            bit_offset,
        )
        .map_err(|error| AcquisitionError::Internal(error.to_string()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BufferedFakePhase {
    Sampling,
    Uploading,
    Finished,
}

#[derive(Debug)]
struct BufferedFakeControlState {
    manual_upload: bool,
    upload_permits: usize,
    stop_requested: bool,
    phase: BufferedFakePhase,
}

#[derive(Debug)]
struct BufferedFakeControl {
    state: Mutex<BufferedFakeControlState>,
    changed: Condvar,
}

impl BufferedFakeControl {
    fn new(manual_upload: bool) -> Self {
        Self {
            state: Mutex::new(BufferedFakeControlState {
                manual_upload,
                upload_permits: 0,
                stop_requested: false,
                phase: BufferedFakePhase::Sampling,
            }),
            changed: Condvar::new(),
        }
    }

    fn begin_upload(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.phase = BufferedFakePhase::Uploading;
        self.changed.notify_all();
    }

    fn finish(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.phase = BufferedFakePhase::Finished;
        self.changed.notify_all();
    }

    fn wait_for_upload_chunk(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        while state.manual_upload && state.upload_permits == 0 && !state.stop_requested {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        if state.stop_requested {
            return false;
        }
        if state.manual_upload {
            state.upload_permits -= 1;
        }
        true
    }

    fn request_stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.stop_requested = true;
        self.changed.notify_all();
    }
}

/// Controller for manually releasing chunks from a [`BufferedFakeProvider`].
#[derive(Clone, Debug)]
pub struct BufferedFakeController {
    control: Arc<BufferedFakeControl>,
}

impl BufferedFakeController {
    /// Waits for sampling to finish and the fake to enter its upload phase.
    ///
    /// # Parameters
    /// - `timeout`: Maximum time to wait for the upload phase.
    pub fn wait_until_upload(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .control
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while state.phase == BufferedFakePhase::Sampling {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, result) = self
                .control
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
            if result.timed_out() && state.phase == BufferedFakePhase::Sampling {
                return false;
            }
        }
        state.phase == BufferedFakePhase::Uploading
    }

    /// Allows this many pending upload chunks to be emitted.
    ///
    /// # Parameters
    /// - `chunks`: Number of chunks to release; zero leaves the provider blocked.
    pub fn grant_upload_chunks(&self, chunks: usize) {
        let mut state = self
            .control
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.upload_permits = state.upload_permits.saturating_add(chunks);
        self.control.changed.notify_all();
    }
}

/// Prepared-acquisition provider that models a device-buffered capture.
pub struct BufferedFakeProvider {
    config: BufferedFakeConfig,
    control: Arc<BufferedFakeControl>,
    buffer_pool: CaptureBufferPool,
}

impl BufferedFakeProvider {
    /// Creates a provider that uploads all chunks without test-controlled pauses.
    ///
    /// # Parameters
    /// - `config`: Validated finite-capture configuration.
    pub fn new(config: BufferedFakeConfig) -> Self {
        Self::with_control(config, Arc::new(BufferedFakeControl::new(false)))
    }

    /// Creates a provider whose upload progression is controlled by a returned controller.
    ///
    /// # Parameters
    /// - `config`: Validated finite-capture configuration.
    pub fn manually_uploaded(config: BufferedFakeConfig) -> (Self, BufferedFakeController) {
        let control = Arc::new(BufferedFakeControl::new(true));
        let provider = Self::with_control(config, Arc::clone(&control));
        (provider, BufferedFakeController { control })
    }

    fn with_control(config: BufferedFakeConfig, control: Arc<BufferedFakeControl>) -> Self {
        let initial_capacity = config
            .maximum_chunk_bytes()
            .expect("validated buffered fake configuration has bounded chunks");
        Self {
            config,
            control,
            buffer_pool: CaptureBufferPool::new(2, initial_capacity)
                .expect("buffered fake uses a non-zero pool size"),
        }
    }

    /// Returns the provider capabilities derived from its configuration.
    pub fn capabilities(&self) -> &CaptureProviderCapabilities {
        self.config.capabilities()
    }

    /// Creates a prepared acquisition and publishes the normal preparation states.
    ///
    /// # Parameters
    /// - `context`: Session context receiving status, chunks, progress, and completion events.
    pub fn prepare(
        self,
        mut context: AcquisitionContext,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        context.publish_status(
            CaptureSessionState::Preparing,
            CaptureAcquisitionPhase::Preparing,
        )?;
        context.publish_status(
            CaptureSessionState::Prepared,
            CaptureAcquisitionPhase::Ready,
        )?;
        Ok(Box::new(PreparedBufferedFakeAcquisition {
            session_id: context.session_id(),
            context: Some(context),
            config: self.config,
            control: self.control,
            buffer_pool: self.buffer_pool,
            handle: None,
            started: false,
        }))
    }
}

struct PreparedBufferedFakeAcquisition {
    session_id: CaptureSessionId,
    context: Option<AcquisitionContext>,
    config: BufferedFakeConfig,
    control: Arc<BufferedFakeControl>,
    buffer_pool: CaptureBufferPool,
    handle: Option<JoinHandle<AcquisitionResult<AcquisitionOutcome>>>,
    started: bool,
}

impl PreparedBufferedFakeAcquisition {
    fn run(
        mut context: AcquisitionContext,
        config: BufferedFakeConfig,
        control: Arc<BufferedFakeControl>,
        buffer_pool: CaptureBufferPool,
    ) -> AcquisitionResult<AcquisitionOutcome> {
        let result = Self::run_inner(&mut context, &config, &control, &buffer_pool);
        control.finish();
        if let Err(error) = &result {
            context.publish_failure(error);
        }
        result
    }

    fn run_inner(
        context: &mut AcquisitionContext,
        config: &BufferedFakeConfig,
        control: &BufferedFakeControl,
        buffer_pool: &CaptureBufferPool,
    ) -> AcquisitionResult<AcquisitionOutcome> {
        let trigger_sample = config.first_trigger_sample();
        if config.has_trigger() {
            context.publish_status(
                CaptureSessionState::Armed,
                CaptureAcquisitionPhase::WaitingForTrigger,
            )?;
            if let Some(trigger_sample) = trigger_sample {
                context.publish_triggered(trigger_sample)?;
                context.publish_status(
                    CaptureSessionState::Triggered,
                    CaptureAcquisitionPhase::CapturingOnDevice,
                )?;
            }
        } else {
            context.publish_status(
                CaptureSessionState::Recording,
                CaptureAcquisitionPhase::CapturingOnDevice,
            )?;
        }

        context.publish_status(
            CaptureSessionState::Recording,
            CaptureAcquisitionPhase::UploadingBufferedData,
        )?;
        control.begin_upload();

        let mut captured_samples = 0_u64;
        let mut transferred_bytes = 0_u64;
        let mut sequence = 0_u64;
        let mut stopped = false;
        while captured_samples < config.total_samples {
            if !control.wait_for_upload_chunk() {
                stopped = true;
                break;
            }
            let sample_count = config
                .upload_chunk_samples
                .min(config.total_samples - captured_samples);
            let chunk = config.build_chunk(
                buffer_pool,
                context.session_id(),
                sequence,
                captured_samples,
                sample_count,
            )?;
            transferred_bytes = transferred_bytes
                .checked_add(chunk.encoded_byte_len() as u64)
                .ok_or_else(|| AcquisitionError::Internal("byte count overflow".into()))?;
            context.append(chunk)?;
            captured_samples += sample_count;
            sequence += 1;
            context.publish_progress(CaptureProgress {
                captured_samples: Some(captured_samples),
                transferred_bytes: Some(transferred_bytes),
            })?;
        }
        context.finish_writer()?;
        context.publish_status(
            CaptureSessionState::Stopping,
            CaptureAcquisitionPhase::Finalizing,
        )?;
        context.publish_status(
            CaptureSessionState::Complete,
            CaptureAcquisitionPhase::Finalizing,
        )?;
        Ok(AcquisitionOutcome {
            session_id: context.session_id(),
            captured_samples,
            chunk_count: sequence,
            stopped,
            completion: if stopped && config.has_trigger() && trigger_sample.is_none() {
                CaptureCompletion::CancelledBeforeTrigger
            } else if stopped {
                CaptureCompletion::Stopped
            } else {
                CaptureCompletion::Finished
            },
        })
    }

    fn join_worker(&mut self) -> AcquisitionResult<AcquisitionOutcome> {
        let handle = self.handle.take().ok_or(AcquisitionError::NotStarted)?;
        handle
            .join()
            .map_err(|_| AcquisitionError::WorkerPanicked)?
    }
}

impl PreparedAcquisition for PreparedBufferedFakeAcquisition {
    fn session_id(&self) -> CaptureSessionId {
        self.session_id
    }

    fn start(&mut self) -> AcquisitionResult<()> {
        if self.started {
            return Err(AcquisitionError::AlreadyStarted);
        }
        let context = self
            .context
            .take()
            .ok_or(AcquisitionError::AlreadyStarted)?;
        let config = self.config.clone();
        let control = Arc::clone(&self.control);
        let buffer_pool = self.buffer_pool.clone();
        self.handle = Some(
            std::thread::Builder::new()
                .name("buffered-live-capture".into())
                .spawn(move || Self::run(context, config, control, buffer_pool))
                .map_err(|error| AcquisitionError::WorkerStart(error.to_string()))?,
        );
        self.started = true;
        Ok(())
    }

    fn request_stop(&self) -> AcquisitionResult<()> {
        self.control.request_stop();
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.handle.as_ref().is_some_and(JoinHandle::is_finished)
    }

    fn join(mut self: Box<Self>) -> AcquisitionResult<AcquisitionOutcome> {
        self.join_worker()
    }
}

impl Drop for PreparedBufferedFakeAcquisition {
    fn drop(&mut self) {
        self.control.request_stop();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
