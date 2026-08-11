use std::collections::VecDeque;
use std::sync::Arc;

use signal_runtime::{
    InputPort, OutputPort, PortDirection, PortPayload, PortSchema, ProcessNode, ProtocolKind,
    RuntimeExecutionMode, Sender, WorkError, WorkOutcome, WorkResult,
};

use super::contracts::{BlockData, CaptureMetadata, packed_bit};
use super::host_protocol::{
    CaptureWorkerMessage, CaptureWorkerReplayBlock, CaptureWorkerReplayRequest,
};
use super::preparation::CaptureIndexPreparationRequest;
use super::worker_client::CaptureWorkerClient;
use crate::{Sample, SampleBlock};

const MAX_REPLAY_PAYLOAD_BYTES: u64 = 32 * 1024 * 1024;
const EDGE_SCAN_SAMPLES_PER_STEP: usize = 64 * 1024;

/// Portable source node that replays bounded blocks from a worker-owned capture session.
pub struct CaptureWorkerReplaySource {
    name: String,
    client: Arc<CaptureWorkerClient>,
    preparation: CaptureIndexPreparationRequest,
    metadata: CaptureMetadata,
    preparation_sequence: Option<u64>,
    replay_sequence: Option<u64>,
    session_id: Option<u64>,
    channels: Option<Vec<ReplayChannel>>,
    pending_blocks: VecDeque<PendingBlock>,
    block: u64,
    start_channel: u64,
    complete: bool,
}

struct ReplayChannel {
    block_senders: Vec<Sender<SampleBlock>>,
    edge_senders: Vec<Sender<Sample>>,
    edge_value: Option<bool>,
}

struct PendingBlock {
    channel: usize,
    data: BlockData,
    start_sample: u64,
    valid_samples: usize,
    block_sent: bool,
    edge_offset: usize,
}

impl CaptureWorkerReplaySource {
    /// Creates a replay source from a prepared worker-backed capture.
    ///
    /// # Parameters
    /// - `name`: Input consumed by this operation.
    /// - `client`: Input consumed by this operation.
    /// - `preparation`: Input consumed by this operation.
    /// - `metadata`: Input consumed by this operation.
    pub fn new(
        name: impl Into<String>,
        client: Arc<CaptureWorkerClient>,
        preparation: CaptureIndexPreparationRequest,
        metadata: CaptureMetadata,
    ) -> Self {
        Self {
            name: name.into(),
            client,
            preparation,
            metadata,
            preparation_sequence: None,
            replay_sequence: None,
            session_id: None,
            channels: None,
            pending_blocks: VecDeque::new(),
            block: 0,
            start_channel: 0,
            complete: false,
        }
    }

    fn step(&mut self, outputs: &[OutputPort]) -> WorkResult<WorkOutcome> {
        if self.complete {
            return Err(WorkError::Shutdown);
        }
        if self.channels.is_none() {
            self.channels = Some(
                outputs
                    .iter()
                    .map(|output| ReplayChannel {
                        block_senders: output.split_senders::<SampleBlock>().unwrap_or_default(),
                        edge_senders: output.split_senders::<Sample>().unwrap_or_default(),
                        edge_value: None,
                    })
                    .collect(),
            );
        }
        if self
            .channels
            .as_ref()
            .is_none_or(|channels| channels.iter().all(ReplayChannel::is_disconnected))
        {
            self.finish();
            return Ok(WorkOutcome::progressed(0));
        }

        if self.session_id.is_none() {
            return self.prepare();
        }
        if let Some(outcome) = self.step_pending_block()? {
            return Ok(outcome);
        }
        if self.block >= self.metadata.total_blocks {
            self.finish();
            return Ok(WorkOutcome::progressed(0));
        }
        self.poll_or_submit_replay()
    }

    fn prepare(&mut self) -> WorkResult<WorkOutcome> {
        let Some(sequence) = self.preparation_sequence else {
            let sequence = self
                .client
                .submit_preparation(self.preparation.clone())
                .map_err(|error| WorkError::NodeError(error.to_string()))?;
            self.preparation_sequence = Some(sequence);
            return Ok(WorkOutcome::progressed(0));
        };
        let updates = self.client.take_updates(sequence);
        if updates.is_empty() {
            return Ok(WorkOutcome::idle());
        }
        for update in updates {
            match update {
                CaptureWorkerMessage::Prepared { session_id, .. } => {
                    self.session_id = Some(session_id);
                    self.preparation_sequence = None;
                }
                CaptureWorkerMessage::Failed { error, .. } => {
                    return Err(WorkError::NodeError(error.to_string()));
                }
                CaptureWorkerMessage::Cancelled { .. } => {
                    return Err(WorkError::NodeError(
                        "capture replay preparation was cancelled".to_owned(),
                    ));
                }
                CaptureWorkerMessage::Metadata { .. } | CaptureWorkerMessage::Progress { .. } => {}
                CaptureWorkerMessage::Window { .. } | CaptureWorkerMessage::Replay { .. } => {
                    return Err(WorkError::NodeError(
                        "capture worker returned an invalid preparation result".to_owned(),
                    ));
                }
            }
        }
        Ok(WorkOutcome::progressed(0))
    }

    fn poll_or_submit_replay(&mut self) -> WorkResult<WorkOutcome> {
        if let Some(sequence) = self.replay_sequence {
            let updates = self.client.take_updates(sequence);
            if updates.is_empty() {
                return Ok(WorkOutcome::idle());
            }
            for update in updates {
                match update {
                    CaptureWorkerMessage::Replay {
                        blocks,
                        next_channel,
                        ..
                    } => {
                        self.accept_blocks(blocks)?;
                        self.start_channel = next_channel;
                        let channel_count = self.active_channels().len() as u64;
                        if self.start_channel >= channel_count {
                            self.start_channel = 0;
                            self.block = self.block.saturating_add(1);
                        }
                        self.replay_sequence = None;
                    }
                    CaptureWorkerMessage::Failed { error, .. } => {
                        return Err(WorkError::NodeError(error.to_string()));
                    }
                    CaptureWorkerMessage::Cancelled { .. } => {
                        return Err(WorkError::NodeError(
                            "capture replay was cancelled".to_owned(),
                        ));
                    }
                    CaptureWorkerMessage::Progress { .. }
                    | CaptureWorkerMessage::Metadata { .. }
                    | CaptureWorkerMessage::Prepared { .. }
                    | CaptureWorkerMessage::Window { .. } => {
                        return Err(WorkError::NodeError(
                            "capture worker returned an invalid replay result".to_owned(),
                        ));
                    }
                }
            }
            return Ok(WorkOutcome::progressed(0));
        }

        let sequence = self
            .client
            .submit_replay(
                self.session_id.expect("prepared above"),
                CaptureWorkerReplayRequest {
                    channels: self.active_channels(),
                    block: self.block,
                    start_channel: self.start_channel,
                    max_payload_bytes: MAX_REPLAY_PAYLOAD_BYTES,
                },
            )
            .map_err(|error| WorkError::NodeError(error.to_string()))?;
        self.replay_sequence = Some(sequence);
        Ok(WorkOutcome::progressed(0))
    }

    fn active_channels(&self) -> Vec<u64> {
        self.channels
            .as_ref()
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(_, channel)| !channel.is_disconnected())
            .map(|(channel, _)| channel as u64)
            .collect()
    }

    fn accept_blocks(&mut self, blocks: Vec<CaptureWorkerReplayBlock>) -> WorkResult<()> {
        for block in blocks {
            let channel = usize::try_from(block.channel).map_err(|_| {
                WorkError::NodeError("capture channel exceeds this host".to_owned())
            })?;
            let valid_samples = usize::try_from(block.valid_samples).map_err(|_| {
                WorkError::NodeError("capture block sample count exceeds this host".to_owned())
            })?;
            self.pending_blocks.push_back(PendingBlock {
                channel,
                data: BlockData::from(block.data),
                start_sample: block.start_sample,
                valid_samples,
                block_sent: false,
                edge_offset: 0,
            });
        }
        Ok(())
    }

    fn step_pending_block(&mut self) -> WorkResult<Option<WorkOutcome>> {
        let Some(mut block) = self.pending_blocks.pop_front() else {
            return Ok(None);
        };
        let timestamp_step = (1_000_000_000.0 / self.metadata.samplerate_hz) as u64;
        let channel = self
            .channels
            .as_mut()
            .and_then(|channels| channels.get_mut(block.channel))
            .ok_or_else(|| WorkError::NodeError("capture replay channel is invalid".to_owned()))?;

        if !block.block_sent {
            let sample_block = SampleBlock::new(
                block.data.clone(),
                block.start_sample,
                block.valid_samples,
                timestamp_step,
            );
            channel
                .block_senders
                .retain(|sender| sender.send(sample_block.clone()).is_ok());
            block.block_sent = true;
            if !channel.block_senders.is_empty() {
                self.pending_blocks.push_back(block);
                return Ok(Some(WorkOutcome::progressed(1)));
            }
        }

        if channel.edge_senders.is_empty() || block.valid_samples == 0 {
            return Ok(Some(WorkOutcome::progressed(0)));
        }
        if channel.edge_value.is_none() {
            let value = packed_bit(&block.data, 0);
            channel.edge_value = Some(value);
            block.edge_offset = 1;
            let sample = Sample::new(value, block.start_sample.saturating_mul(timestamp_step));
            channel
                .edge_senders
                .retain(|sender| sender.send(sample).is_ok());
            self.pending_blocks.push_back(block);
            return Ok(Some(WorkOutcome::progressed(1)));
        }

        let end = block
            .edge_offset
            .saturating_add(EDGE_SCAN_SAMPLES_PER_STEP)
            .min(block.valid_samples);
        for offset in block.edge_offset..end {
            let value = packed_bit(&block.data, offset);
            if Some(value) != channel.edge_value {
                channel.edge_value = Some(value);
                block.edge_offset = offset + 1;
                let sample_position = block.start_sample.saturating_add(offset as u64);
                let sample = Sample::new(value, sample_position.saturating_mul(timestamp_step));
                channel
                    .edge_senders
                    .retain(|sender| sender.send(sample).is_ok());
                self.pending_blocks.push_back(block);
                return Ok(Some(WorkOutcome::progressed(1)));
            }
        }
        block.edge_offset = end;
        if end < block.valid_samples {
            self.pending_blocks.push_back(block);
        }
        Ok(Some(WorkOutcome::progressed(0)))
    }

    fn finish(&mut self) {
        if let Some(channels) = self.channels.as_ref() {
            for channel in channels {
                for sender in &channel.block_senders {
                    sender.close();
                }
                for sender in &channel.edge_senders {
                    sender.close();
                }
            }
        }
        self.complete = true;
    }
}

impl ReplayChannel {
    fn is_disconnected(&self) -> bool {
        self.block_senders.is_empty() && self.edge_senders.is_empty()
    }
}

impl ProcessNode for CaptureWorkerReplaySource {
    fn name(&self) -> &str {
        &self.name
    }

    fn should_stop(&self) -> bool {
        self.complete
    }

    fn set_runtime_execution_mode(&mut self, _mode: RuntimeExecutionMode) {}

    fn num_inputs(&self) -> usize {
        0
    }

    fn num_outputs(&self) -> usize {
        self.metadata.total_probes
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        (0..self.metadata.total_probes)
            .map(|channel| {
                PortSchema::state::<Sample>(format!("ch{channel}"), channel, PortDirection::Output)
                    .with_protocols(vec![ProtocolKind::Stream])
                    .with_payloads(vec![
                        PortPayload::new::<SampleBlock>().with_default_buffer_capacity(2),
                        PortPayload::new::<Sample>().state(),
                    ])
            })
            .collect()
    }

    fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        self.step(outputs).map(WorkOutcome::produced_items)
    }

    fn work_outcome(
        &mut self,
        _inputs: &[InputPort],
        outputs: &[OutputPort],
    ) -> WorkResult<WorkOutcome> {
        self.step(outputs)
    }
}

impl Drop for CaptureWorkerReplaySource {
    fn drop(&mut self) {
        if let Some(sequence) = self.preparation_sequence.take() {
            self.client.cancel(sequence);
        }
        if let Some(sequence) = self.replay_sequence.take() {
            self.client.cancel(sequence);
        }
        if let Some(session_id) = self.session_id.take() {
            self.client.release(session_id);
        }
    }
}

#[cfg(test)]
mod worker_replay_source_tests {
    use crossbeam_channel::bounded;
    use platform_artifacts::SourceIdentity;
    use platform_runtime::WorkerOperation;
    use signal_runtime::{ChannelMessage, Watchdog};

    use super::*;

    fn metadata() -> CaptureMetadata {
        CaptureMetadata {
            total_probes: 1,
            samplerate: "1 GHz".to_owned(),
            samplerate_hz: 1_000_000_000.0,
            sample_period: 0.000_000_001,
            total_samples: 8,
            total_blocks: 1,
            samples_per_block: 8,
            probe_names: vec!["D0".to_owned()],
            trigger_sample: None,
        }
    }

    #[test]
    fn source_prepares_replays_and_publishes_a_bounded_block() {
        let client = Arc::new(CaptureWorkerClient::new(2).unwrap());
        let mut source = CaptureWorkerReplaySource::new(
            "worker source",
            Arc::clone(&client),
            CaptureIndexPreparationRequest::new(
                WorkerOperation::new("test.capture.prepare/v1").unwrap(),
                vec![1],
            ),
            metadata(),
        );
        let (sender, receiver) = bounded(4);
        let output = OutputPort::new_with_watchdog(
            Sender::<SampleBlock>::new(vec![sender]),
            &Watchdog::new(),
            "worker source",
            "ch0",
        );

        source
            .work_outcome(&[], std::slice::from_ref(&output))
            .unwrap();
        let requests = client.drain_requests();
        let [super::super::host_protocol::CaptureWorkerRequest::Prepare { sequence, .. }] =
            requests.as_slice()
        else {
            panic!("source must request preparation first");
        };
        let preparation_sequence = *sequence;
        client
            .publish(CaptureWorkerMessage::Metadata {
                sequence: preparation_sequence,
                metadata: metadata(),
            })
            .unwrap();
        client
            .publish(CaptureWorkerMessage::Prepared {
                sequence: preparation_sequence,
                session_id: 7,
                display_name: "fixture.dsl".to_owned(),
                source_identity: SourceIdentity::from_bytes([2; 32]),
                index_identity: SourceIdentity::from_bytes([3; 32]),
                metadata: metadata(),
            })
            .unwrap();

        source
            .work_outcome(&[], std::slice::from_ref(&output))
            .unwrap();
        source
            .work_outcome(&[], std::slice::from_ref(&output))
            .unwrap();
        let requests = client.drain_requests();
        let [super::super::host_protocol::CaptureWorkerRequest::Replay { sequence, .. }] =
            requests.as_slice()
        else {
            panic!("prepared source must request replay");
        };
        let replay_sequence = *sequence;
        client
            .publish(CaptureWorkerMessage::Replay {
                sequence: replay_sequence,
                block: 0,
                blocks: vec![CaptureWorkerReplayBlock {
                    channel: 0,
                    block: 0,
                    start_sample: 0,
                    valid_samples: 8,
                    data: vec![0b1010_0101],
                }],
                next_channel: 1,
            })
            .unwrap();

        source
            .work_outcome(&[], std::slice::from_ref(&output))
            .unwrap();
        source
            .work_outcome(&[], std::slice::from_ref(&output))
            .unwrap();
        assert!(matches!(
            receiver.try_recv(),
            Ok(ChannelMessage::Sample(block))
                if *block.data == [0b1010_0101] && block.num_samples == 8
        ));
    }
}
