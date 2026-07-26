use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use signal_processing::{
    AcquisitionContext, AcquisitionError, AcquisitionOutcome, AcquisitionResult,
    CaptureAcquisitionPhase, CaptureChannelId, CaptureChunk, CaptureCompletion,
    CaptureDataDelivery, CaptureProgress, CaptureProviderCapabilities, CaptureSessionId,
    CaptureSessionState, CaptureSettingCombination, PreparedAcquisition, SimpleTriggerCondition,
};

#[derive(Clone)]
struct ScriptedConfig {
    channels: Arc<[CaptureChannelId]>,
    chunk_sample_counts: Arc<[u64]>,
    trigger_sample: Option<u64>,
    seed: u64,
    delivery: CaptureDataDelivery,
}

impl ScriptedConfig {
    fn total_samples(&self) -> u64 {
        self.chunk_sample_counts.iter().sum()
    }

    fn level_at(&self, sample: u64, channel: usize) -> bool {
        let channel = channel as u64;
        if self.delivery == CaptureDataDelivery::BufferedUpload {
            let period = channel.wrapping_mul(2).wrapping_add(3);
            return ((sample / period) ^ channel ^ self.seed) & 1 != 0;
        }
        let mixed = sample
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .rotate_left((channel % 63) as u32)
            ^ channel.wrapping_mul(0xd6e8_feb8_6659_fd93)
            ^ self.seed;
        (mixed ^ (mixed >> 17) ^ (mixed >> 41)) & 1 != 0
    }

    fn build_chunk(
        &self,
        session_id: CaptureSessionId,
        sequence: u64,
        start_sample: u64,
        sample_count: u64,
    ) -> AcquisitionResult<CaptureChunk> {
        let bit_offset = ((sequence * 3 + 1) % 8) as u8;
        let bit_count = sample_count as usize * self.channels.len();
        let mut bytes = vec![0_u8; (usize::from(bit_offset) + bit_count).div_ceil(8)];
        for relative_sample in 0..sample_count {
            for channel in 0..self.channels.len() {
                if self.level_at(start_sample + relative_sample, channel) {
                    let bit = usize::from(bit_offset)
                        + relative_sample as usize * self.channels.len()
                        + channel;
                    bytes[bit / 8] |= 1 << (bit % 8);
                }
            }
        }
        CaptureChunk::packed_lsb_first(
            session_id,
            sequence,
            start_sample,
            sample_count,
            Arc::clone(&self.channels),
            bytes,
            bit_offset,
        )
        .map_err(|error| AcquisitionError::Internal(error.to_string()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScriptedPhase {
    Waiting,
    Uploading,
    Finished,
}

struct ScriptedControlState {
    permits: usize,
    stop_requested: bool,
    abort_requested: bool,
    force_trigger_requested: bool,
    phase: ScriptedPhase,
}

struct ScriptedControl {
    state: Mutex<ScriptedControlState>,
    changed: Condvar,
}

enum ScriptedWake {
    Permit,
    Stop,
    ForceTrigger,
}

impl ScriptedControl {
    fn new() -> Self {
        Self {
            state: Mutex::new(ScriptedControlState {
                permits: 0,
                stop_requested: false,
                abort_requested: false,
                force_trigger_requested: false,
                phase: ScriptedPhase::Waiting,
            }),
            changed: Condvar::new(),
        }
    }

    fn wait_for_permit(&self, accept_force_trigger: bool) -> ScriptedWake {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        while state.permits == 0
            && !state.stop_requested
            && !state.abort_requested
            && !(accept_force_trigger && state.force_trigger_requested)
        {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        if state.stop_requested || state.abort_requested {
            return ScriptedWake::Stop;
        }
        if accept_force_trigger && state.force_trigger_requested {
            state.force_trigger_requested = false;
            return ScriptedWake::ForceTrigger;
        }
        state.permits -= 1;
        ScriptedWake::Permit
    }

    fn grant(&self, chunks: usize) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.permits = state.permits.saturating_add(chunks);
        self.changed.notify_all();
    }

    fn request_stop(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.stop_requested = true;
        self.changed.notify_all();
    }

    fn request_abort(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.abort_requested = true;
        self.changed.notify_all();
    }

    fn request_force_trigger(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.force_trigger_requested = true;
        self.changed.notify_all();
    }

    fn set_phase(&self, phase: ScriptedPhase) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.phase = phase;
        self.changed.notify_all();
    }

    fn aborted(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .abort_requested
    }

    fn wait_until_upload(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        while state.phase == ScriptedPhase::Waiting {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, result) = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
            if result.timed_out() && state.phase == ScriptedPhase::Waiting {
                return false;
            }
        }
        state.phase == ScriptedPhase::Uploading
    }
}

struct ScriptedProvider {
    config: ScriptedConfig,
    control: Arc<ScriptedControl>,
}

impl ScriptedProvider {
    fn prepare(
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
        Ok(Box::new(ScriptedPreparedAcquisition {
            session_id: context.session_id(),
            context: Some(context),
            config: self.config,
            control: self.control,
            handle: None,
        }))
    }
}

struct ScriptedPreparedAcquisition {
    session_id: CaptureSessionId,
    context: Option<AcquisitionContext>,
    config: ScriptedConfig,
    control: Arc<ScriptedControl>,
    handle: Option<JoinHandle<AcquisitionResult<AcquisitionOutcome>>>,
}

impl ScriptedPreparedAcquisition {
    fn run(
        mut context: AcquisitionContext,
        config: ScriptedConfig,
        control: Arc<ScriptedControl>,
    ) -> AcquisitionResult<AcquisitionOutcome> {
        let result = match config.delivery {
            CaptureDataDelivery::DuringAcquisition => {
                Self::run_streaming(&mut context, &config, &control)
            }
            CaptureDataDelivery::BufferedUpload => {
                Self::run_buffered(&mut context, &config, &control)
            }
        };
        control.set_phase(ScriptedPhase::Finished);
        if let Err(error) = &result {
            context.publish_failure(error);
        }
        result
    }

    fn run_streaming(
        context: &mut AcquisitionContext,
        config: &ScriptedConfig,
        control: &ScriptedControl,
    ) -> AcquisitionResult<AcquisitionOutcome> {
        let mut triggered = config.trigger_sample.is_none();
        context.publish_status(
            if triggered {
                CaptureSessionState::Recording
            } else {
                CaptureSessionState::Armed
            },
            if triggered {
                CaptureAcquisitionPhase::ReceivingLiveData
            } else {
                CaptureAcquisitionPhase::WaitingForTrigger
            },
        )?;
        let mut captured_samples = 0_u64;
        let mut transferred_bytes = 0_u64;
        let mut chunk_count = 0_u64;
        let mut stopped = false;
        for (sequence, sample_count) in config.chunk_sample_counts.iter().copied().enumerate() {
            loop {
                match control.wait_for_permit(true) {
                    ScriptedWake::Permit => break,
                    ScriptedWake::Stop => {
                        stopped = true;
                        break;
                    }
                    ScriptedWake::ForceTrigger if !triggered => {
                        triggered = true;
                        Self::publish_trigger(context, captured_samples)?;
                    }
                    ScriptedWake::ForceTrigger => {}
                }
            }
            if stopped {
                break;
            }
            let chunk = config.build_chunk(
                context.session_id(),
                sequence as u64,
                captured_samples,
                sample_count,
            )?;
            transferred_bytes = transferred_bytes.saturating_add(chunk.encoded_byte_len() as u64);
            context.append(chunk)?;
            if !triggered
                && let Some(trigger_sample) = config.trigger_sample
                && trigger_sample >= captured_samples
                && trigger_sample < captured_samples + sample_count
            {
                triggered = true;
                Self::publish_trigger(context, trigger_sample)?;
            }
            captured_samples += sample_count;
            chunk_count += 1;
            context.publish_progress(CaptureProgress {
                captured_samples: Some(captured_samples),
                transferred_bytes: Some(transferred_bytes),
            })?;
        }
        Self::finish(
            context,
            control,
            config.trigger_sample.is_some(),
            triggered,
            captured_samples,
            chunk_count,
            stopped,
        )
    }

    fn run_buffered(
        context: &mut AcquisitionContext,
        config: &ScriptedConfig,
        control: &ScriptedControl,
    ) -> AcquisitionResult<AcquisitionOutcome> {
        let triggered = config.trigger_sample.is_some();
        if let Some(trigger_sample) = config.trigger_sample {
            context.publish_status(
                CaptureSessionState::Armed,
                CaptureAcquisitionPhase::WaitingForTrigger,
            )?;
            context.publish_triggered(trigger_sample)?;
            context.publish_status(
                CaptureSessionState::Triggered,
                CaptureAcquisitionPhase::CapturingOnDevice,
            )?;
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
        control.set_phase(ScriptedPhase::Uploading);
        let mut captured_samples = 0_u64;
        let mut transferred_bytes = 0_u64;
        let mut chunk_count = 0_u64;
        let mut stopped = false;
        for (sequence, sample_count) in config.chunk_sample_counts.iter().copied().enumerate() {
            match control.wait_for_permit(false) {
                ScriptedWake::Permit => {}
                ScriptedWake::Stop => {
                    stopped = true;
                    break;
                }
                ScriptedWake::ForceTrigger => unreachable!(),
            }
            let chunk = config.build_chunk(
                context.session_id(),
                sequence as u64,
                captured_samples,
                sample_count,
            )?;
            transferred_bytes = transferred_bytes.saturating_add(chunk.encoded_byte_len() as u64);
            context.append(chunk)?;
            captured_samples += sample_count;
            chunk_count += 1;
            context.publish_progress(CaptureProgress {
                captured_samples: Some(captured_samples),
                transferred_bytes: Some(transferred_bytes),
            })?;
        }
        Self::finish(
            context,
            control,
            config.trigger_sample.is_some(),
            triggered,
            captured_samples,
            chunk_count,
            stopped,
        )
    }

    fn publish_trigger(context: &mut AcquisitionContext, sample: u64) -> AcquisitionResult<()> {
        context.publish_triggered(sample)?;
        context.publish_status(
            CaptureSessionState::Triggered,
            CaptureAcquisitionPhase::ReceivingLiveData,
        )?;
        context.publish_status(
            CaptureSessionState::Recording,
            CaptureAcquisitionPhase::ReceivingLiveData,
        )
    }

    fn finish(
        context: &mut AcquisitionContext,
        control: &ScriptedControl,
        has_trigger: bool,
        triggered: bool,
        captured_samples: u64,
        chunk_count: u64,
        stopped: bool,
    ) -> AcquisitionResult<AcquisitionOutcome> {
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
            chunk_count,
            stopped,
            completion: if control.aborted() {
                CaptureCompletion::Aborted
            } else if stopped && has_trigger && !triggered {
                CaptureCompletion::CancelledBeforeTrigger
            } else if stopped {
                CaptureCompletion::Stopped
            } else {
                CaptureCompletion::Finished
            },
        })
    }

    fn join_worker(&mut self) -> AcquisitionResult<AcquisitionOutcome> {
        self.handle
            .take()
            .ok_or(AcquisitionError::NotStarted)?
            .join()
            .map_err(|_| AcquisitionError::WorkerPanicked)?
    }
}

impl PreparedAcquisition for ScriptedPreparedAcquisition {
    fn session_id(&self) -> CaptureSessionId {
        self.session_id
    }

    fn start(&mut self) -> AcquisitionResult<()> {
        let context = self
            .context
            .take()
            .ok_or(AcquisitionError::AlreadyStarted)?;
        let config = self.config.clone();
        let control = Arc::clone(&self.control);
        self.handle = Some(
            std::thread::Builder::new()
                .name("ui-test-acquisition".into())
                .spawn(move || Self::run(context, config, control))
                .map_err(|error| AcquisitionError::WorkerStart(error.to_string()))?,
        );
        Ok(())
    }

    fn request_stop(&self) -> AcquisitionResult<()> {
        self.control.request_stop();
        Ok(())
    }

    fn request_abort(&self) -> AcquisitionResult<()> {
        self.control.request_abort();
        Ok(())
    }

    fn request_force_trigger(&self) -> AcquisitionResult<()> {
        self.control.request_force_trigger();
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.handle.as_ref().is_some_and(JoinHandle::is_finished)
    }

    fn join(mut self: Box<Self>) -> AcquisitionResult<AcquisitionOutcome> {
        self.join_worker()
    }
}

impl Drop for ScriptedPreparedAcquisition {
    fn drop(&mut self) {
        self.control.request_stop();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(crate) struct DeterministicFakeConfig {
    config: ScriptedConfig,
    trigger_conditions: Arc<[Option<SimpleTriggerCondition>]>,
}

impl DeterministicFakeConfig {
    pub(crate) fn new(
        channels: Vec<CaptureChannelId>,
        chunk_sample_counts: Vec<u64>,
        seed: u64,
    ) -> AcquisitionResult<Self> {
        if channels.is_empty() || chunk_sample_counts.is_empty() || chunk_sample_counts.contains(&0)
        {
            return Err(AcquisitionError::InvalidRequest(
                "UI test acquisition requires channels and non-zero chunks".into(),
            ));
        }
        let channel_count = channels.len();
        Ok(Self {
            config: ScriptedConfig {
                channels: channels.into(),
                chunk_sample_counts: chunk_sample_counts.into(),
                trigger_sample: None,
                seed,
                delivery: CaptureDataDelivery::DuringAcquisition,
            },
            trigger_conditions: vec![None; channel_count].into(),
        })
    }

    pub(crate) fn total_samples(&self) -> u64 {
        self.config.total_samples()
    }

    pub(crate) fn with_simple_trigger(
        mut self,
        conditions: Vec<Option<SimpleTriggerCondition>>,
    ) -> AcquisitionResult<Self> {
        if conditions.len() != self.config.channels.len() {
            return Err(AcquisitionError::InvalidRequest(
                "UI test trigger channel count mismatch".into(),
            ));
        }
        self.trigger_conditions = conditions.into();
        self.config.trigger_sample = self.first_matching_sample();
        Ok(self)
    }

    pub(crate) fn first_trigger_sample(&self) -> Option<u64> {
        self.config.trigger_sample
    }

    fn first_matching_sample(&self) -> Option<u64> {
        let active = self
            .trigger_conditions
            .iter()
            .flatten()
            .any(|condition| *condition != SimpleTriggerCondition::Ignore);
        active.then(|| {
            (0..self.total_samples()).find(|sample| {
                self.trigger_conditions
                    .iter()
                    .enumerate()
                    .all(|(channel, condition)| {
                        let Some(condition) = condition else {
                            return true;
                        };
                        let previous = sample
                            .checked_sub(1)
                            .map(|previous| self.config.level_at(previous, channel));
                        condition.matches(previous, self.config.level_at(*sample, channel))
                    })
            })
        })?
    }
}

#[derive(Clone)]
pub(crate) struct DeterministicFakeController {
    control: Arc<ScriptedControl>,
}

impl DeterministicFakeController {
    pub(crate) fn grant_chunks(&self, chunks: usize) {
        self.control.grant(chunks);
    }
}

pub(crate) struct DeterministicFakeProvider {
    provider: ScriptedProvider,
}

impl DeterministicFakeProvider {
    pub(crate) fn manually_paced(
        config: DeterministicFakeConfig,
    ) -> (Self, DeterministicFakeController) {
        let control = Arc::new(ScriptedControl::new());
        (
            Self {
                provider: ScriptedProvider {
                    config: config.config,
                    control: Arc::clone(&control),
                },
            },
            DeterministicFakeController { control },
        )
    }

    pub(crate) fn prepare(
        self,
        context: AcquisitionContext,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        self.provider.prepare(context)
    }
}

pub(crate) struct BufferedFakeConfig {
    config: ScriptedConfig,
    capabilities: CaptureProviderCapabilities,
    trigger_conditions: Arc<[Option<SimpleTriggerCondition>]>,
}

impl BufferedFakeConfig {
    pub(crate) fn new(
        channels: Vec<CaptureChannelId>,
        sample_rate_hz: u64,
        total_samples: u64,
        upload_chunk_samples: u64,
        seed: u64,
    ) -> AcquisitionResult<Self> {
        if channels.is_empty()
            || sample_rate_hz == 0
            || total_samples == 0
            || upload_chunk_samples == 0
        {
            return Err(AcquisitionError::InvalidRequest(
                "UI buffered test acquisition requires non-zero settings".into(),
            ));
        }
        let mut setting_matrix = vec![
            CaptureSettingCombination::new(channels.clone(), Arc::from([sample_rate_hz]))
                .map_err(AcquisitionError::InvalidRequest)?,
        ];
        if channels.len() > 1 {
            setting_matrix.push(
                CaptureSettingCombination::new(
                    channels.iter().step_by(2).cloned().collect::<Vec<_>>(),
                    Arc::from([sample_rate_hz.saturating_mul(4)]),
                )
                .map_err(AcquisitionError::InvalidRequest)?,
            );
        }
        let capabilities = CaptureProviderCapabilities::new(
            CaptureDataDelivery::BufferedUpload,
            setting_matrix,
            false,
        )
        .map_err(AcquisitionError::InvalidRequest)?;
        let chunk_sample_counts = (0..total_samples)
            .step_by(upload_chunk_samples as usize)
            .map(|start| upload_chunk_samples.min(total_samples - start))
            .collect::<Vec<_>>();
        let channel_count = channels.len();
        Ok(Self {
            config: ScriptedConfig {
                channels: channels.into(),
                chunk_sample_counts: chunk_sample_counts.into(),
                trigger_sample: None,
                seed,
                delivery: CaptureDataDelivery::BufferedUpload,
            },
            capabilities,
            trigger_conditions: vec![None; channel_count].into(),
        })
    }

    pub(crate) fn with_simple_trigger(
        mut self,
        conditions: Vec<Option<SimpleTriggerCondition>>,
    ) -> AcquisitionResult<Self> {
        if conditions.len() != self.config.channels.len() {
            return Err(AcquisitionError::InvalidRequest(
                "UI buffered test trigger channel count mismatch".into(),
            ));
        }
        self.trigger_conditions = conditions.into();
        self.config.trigger_sample = (0..self.config.total_samples()).find(|sample| {
            self.trigger_conditions
                .iter()
                .enumerate()
                .all(|(channel, condition)| {
                    let Some(condition) = condition else {
                        return true;
                    };
                    let previous = sample
                        .checked_sub(1)
                        .map(|previous| self.config.level_at(previous, channel));
                    condition.matches(previous, self.config.level_at(*sample, channel))
                })
        });
        Ok(self)
    }

    pub(crate) fn capabilities(&self) -> &CaptureProviderCapabilities {
        &self.capabilities
    }

    pub(crate) fn first_trigger_sample(&self) -> Option<u64> {
        self.config.trigger_sample
    }
}

#[derive(Clone)]
pub(crate) struct BufferedFakeController {
    control: Arc<ScriptedControl>,
}

impl BufferedFakeController {
    pub(crate) fn wait_until_upload(&self, timeout: Duration) -> bool {
        self.control.wait_until_upload(timeout)
    }

    pub(crate) fn grant_upload_chunks(&self, chunks: usize) {
        self.control.grant(chunks);
    }
}

pub(crate) struct BufferedFakeProvider {
    provider: ScriptedProvider,
}

impl BufferedFakeProvider {
    pub(crate) fn manually_uploaded(config: BufferedFakeConfig) -> (Self, BufferedFakeController) {
        let control = Arc::new(ScriptedControl::new());
        (
            Self {
                provider: ScriptedProvider {
                    config: config.config,
                    control: Arc::clone(&control),
                },
            },
            BufferedFakeController { control },
        )
    }

    pub(crate) fn prepare(
        self,
        context: AcquisitionContext,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>> {
        self.provider.prepare(context)
    }
}
