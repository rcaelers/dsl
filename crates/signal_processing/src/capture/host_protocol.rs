use serde::{Deserialize, Serialize};

use signal_artifacts::SourceIdentity;

use super::implementation::{CaptureIndexBuildProgress, CaptureMetadata, CaptureSampledWindow};
use super::preparation::CaptureIndexPreparationRequest;
use super::query::CaptureIndexQuery;

/// One bounded request for packed raw blocks from a prepared capture session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureWorkerReplayRequest {
    /// Raw channels requested from the prepared capture.
    pub channels: Vec<u64>,
    /// Capture block number requested for every channel.
    pub block: u64,
    /// First channel to include when resuming a bounded response.
    pub start_channel: u64,
    /// Maximum packed payload bytes permitted in the response.
    pub max_payload_bytes: u64,
}

/// One packed channel block returned by a capture worker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureWorkerReplayBlock {
    /// Raw channel represented by this packed block.
    pub channel: u64,
    /// Capture block number represented by this payload.
    pub block: u64,
    /// First sample index contained in the block.
    pub start_sample: u64,
    /// Number of valid samples in `data`.
    pub valid_samples: u64,
    /// Packed raw sample bytes.
    pub data: Vec<u8>,
}

/// Owned command envelope for a stateful capture-index worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureWorkerRequest {
    /// Prepares a source and creates a worker-owned capture-index session.
    Prepare {
        /// Caller-assigned sequence used to correlate messages.
        sequence: u64,
        /// Source preparation request and host-neutral input description.
        request: CaptureIndexPreparationRequest,
    },
    /// Queries an already prepared index session.
    Query {
        /// Caller-assigned sequence used to correlate messages.
        sequence: u64,
        /// Worker-owned prepared session to query.
        session_id: u64,
        /// Bounded index query.
        query: CaptureIndexQuery,
    },
    /// Replays bounded packed raw capture data from a prepared session.
    Replay {
        /// Caller-assigned sequence used to correlate messages.
        sequence: u64,
        /// Worker-owned prepared session to replay.
        session_id: u64,
        /// Bounded block and channel request.
        request: CaptureWorkerReplayRequest,
    },
    /// Cancels preparation, query, or replay work for one sequence.
    Cancel {
        /// Caller-assigned sequence to cancel.
        sequence: u64,
    },
    /// Releases a worker-owned prepared capture session.
    Release {
        /// Worker-owned session to release.
        session_id: u64,
    },
}

/// Owned result envelope returned by a stateful capture-index worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureWorkerMessage {
    /// Reports source preparation or index-building progress.
    Progress {
        /// Caller-assigned sequence of the active preparation.
        sequence: u64,
        /// Latest progress reported by source preparation.
        progress: CaptureIndexBuildProgress,
    },
    /// Reports immutable source metadata discovered during preparation.
    Metadata {
        /// Caller-assigned sequence of the active preparation.
        sequence: u64,
        /// Discovered capture metadata.
        metadata: CaptureMetadata,
    },
    /// Confirms preparation and provides a worker-owned query session.
    Prepared {
        /// Caller-assigned preparation sequence.
        sequence: u64,
        /// Worker-owned session for later queries and replay.
        session_id: u64,
        /// User-facing source display name.
        display_name: String,
        /// Identity of the raw source content.
        source_identity: SourceIdentity,
        /// Identity of the prepared capture index.
        index_identity: SourceIdentity,
        /// Final source metadata.
        metadata: CaptureMetadata,
    },
    /// Returns a bounded sampled index window.
    Window {
        /// Caller-assigned query sequence.
        sequence: u64,
        /// Sampled index result.
        window: CaptureSampledWindow,
    },
    /// Returns bounded packed raw replay blocks.
    Replay {
        /// Caller-assigned replay sequence.
        sequence: u64,
        /// Replay block number served by this response.
        block: u64,
        /// Packed channel blocks included in this response.
        blocks: Vec<CaptureWorkerReplayBlock>,
        /// Channel at which a subsequent bounded request should resume.
        next_channel: u64,
    },
    /// Reports a worker-side failure without exposing host error types.
    Failed {
        /// Caller-assigned sequence that failed.
        sequence: u64,
        /// User-presentable failure explanation.
        message: String,
    },
    /// Confirms cancellation of one sequence.
    Cancelled {
        /// Caller-assigned sequence that was cancelled.
        sequence: u64,
    },
}

const MESSAGE_BATCH_MAGIC: &[u8; 5] = b"LCWM\x01";
const JSON_MESSAGE: u8 = 0;
const REPLAY_MESSAGE: u8 = 1;

/// Encodes worker results without expanding packed replay bytes through JSON.
///
/// # Parameters
/// - `messages`: Ordered worker results to encode as one transport frame.
pub fn encode_capture_worker_messages(
    messages: &[CaptureWorkerMessage],
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    output.extend_from_slice(MESSAGE_BATCH_MAGIC);
    put_u32(
        &mut output,
        u32::try_from(messages.len())
            .map_err(|_| "capture-worker message batch is too large".to_owned())?,
    );
    for message in messages {
        let mut encoded = Vec::new();
        match message {
            CaptureWorkerMessage::Replay {
                sequence,
                block,
                blocks,
                next_channel,
            } => {
                encoded.push(REPLAY_MESSAGE);
                put_u64(&mut encoded, *sequence);
                put_u64(&mut encoded, *block);
                put_u64(&mut encoded, *next_channel);
                put_u64(&mut encoded, blocks.len() as u64);
                for block in blocks {
                    put_u64(&mut encoded, block.channel);
                    put_u64(&mut encoded, block.block);
                    put_u64(&mut encoded, block.start_sample);
                    put_u64(&mut encoded, block.valid_samples);
                    put_u64(&mut encoded, block.data.len() as u64);
                    encoded.extend_from_slice(&block.data);
                }
            }
            _ => {
                encoded.push(JSON_MESSAGE);
                encoded.extend_from_slice(&serde_json::to_vec(message).map_err(|error| {
                    format!("could not encode capture-worker message: {error}")
                })?);
            }
        }
        put_u64(&mut output, encoded.len() as u64);
        output.extend_from_slice(&encoded);
    }
    Ok(output)
}

/// Decodes a framed batch produced by [`encode_capture_worker_messages`].
pub fn decode_capture_worker_messages(bytes: &[u8]) -> Result<Vec<CaptureWorkerMessage>, String> {
    let mut reader = MessageReader::new(bytes);
    if reader.take(MESSAGE_BATCH_MAGIC.len())? != MESSAGE_BATCH_MAGIC {
        return Err("capture-worker message batch has an invalid header".to_owned());
    }
    let count = reader.u32()? as usize;
    let mut messages = Vec::with_capacity(count);
    for _ in 0..count {
        let length = reader.length()?;
        let mut message = MessageReader::new(reader.take(length)?);
        match message.u8()? {
            JSON_MESSAGE => {
                messages.push(
                    serde_json::from_slice(message.remaining()).map_err(|error| {
                        format!("capture-worker message contains invalid JSON: {error}")
                    })?,
                );
                message.consume_remaining();
            }
            REPLAY_MESSAGE => {
                let sequence = message.u64()?;
                let block = message.u64()?;
                let next_channel = message.u64()?;
                let block_count = message.length()?;
                let mut blocks = Vec::with_capacity(block_count);
                for _ in 0..block_count {
                    let channel = message.u64()?;
                    let block_number = message.u64()?;
                    let start_sample = message.u64()?;
                    let valid_samples = message.u64()?;
                    let data_length = message.length()?;
                    blocks.push(CaptureWorkerReplayBlock {
                        channel,
                        block: block_number,
                        start_sample,
                        valid_samples,
                        data: message.take(data_length)?.to_vec(),
                    });
                }
                messages.push(CaptureWorkerMessage::Replay {
                    sequence,
                    block,
                    blocks,
                    next_channel,
                });
            }
            _ => return Err("capture-worker message has an unknown encoding".to_owned()),
        }
        message.finish()?;
    }
    reader.finish()?;
    Ok(messages)
}

fn put_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

struct MessageReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> MessageReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn length(&mut self) -> Result<usize, String> {
        usize::try_from(self.u64()?)
            .map_err(|_| "capture-worker message length exceeds this host".to_owned())
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or_else(|| "capture-worker message length overflow".to_owned())?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or_else(|| "capture-worker message is truncated".to_owned())?;
        self.cursor = end;
        Ok(value)
    }

    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.cursor..]
    }

    fn consume_remaining(&mut self) {
        self.cursor = self.bytes.len();
    }

    fn finish(self) -> Result<(), String> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err("capture-worker message contains trailing bytes".to_owned())
        }
    }
}

#[cfg(test)]
mod host_protocol_tests {
    use super::*;
    use crate::{CaptureSampledChannel, WorkerOperation};

    fn metadata() -> CaptureMetadata {
        CaptureMetadata {
            total_probes: 2,
            samplerate: "50 MHz".to_owned(),
            samplerate_hz: 50_000_000.0,
            sample_period: 0.000_000_02,
            total_samples: 12_782_165_248,
            total_blocks: 762,
            samples_per_block: 16_777_216,
            probe_names: vec!["D0".to_owned(), "D1".to_owned()],
            trigger_sample: Some(6_000_000_000),
        }
    }

    #[test]
    fn worker_requests_round_trip_without_platform_handles() {
        let messages = [
            CaptureWorkerRequest::Prepare {
                sequence: 1,
                request: CaptureIndexPreparationRequest::new(
                    WorkerOperation::new("test.capture.prepare/v1").unwrap(),
                    vec![1, 2, 3],
                ),
            },
            CaptureWorkerRequest::Query {
                sequence: 2,
                session_id: 9,
                query: CaptureIndexQuery {
                    channels: vec![0, 7],
                    start_sample: 4_000_000_000,
                    end_sample: 4_500_000_000,
                    target_points: 1_920,
                },
            },
            CaptureWorkerRequest::Replay {
                sequence: 3,
                session_id: 9,
                request: CaptureWorkerReplayRequest {
                    channels: vec![0, 7],
                    block: 500,
                    start_channel: 1,
                    max_payload_bytes: 32 * 1024 * 1024,
                },
            },
            CaptureWorkerRequest::Cancel { sequence: 2 },
            CaptureWorkerRequest::Release { session_id: 9 },
        ];

        for message in messages {
            let encoded = serde_json::to_vec(&message).unwrap();
            assert_eq!(
                serde_json::from_slice::<CaptureWorkerRequest>(&encoded).unwrap(),
                message
            );
        }
    }

    #[test]
    fn worker_results_preserve_large_capture_coordinates() {
        let messages = [
            CaptureWorkerMessage::Progress {
                sequence: 1,
                progress: CaptureIndexBuildProgress {
                    completed: 381,
                    total: 762,
                },
            },
            CaptureWorkerMessage::Metadata {
                sequence: 1,
                metadata: metadata(),
            },
            CaptureWorkerMessage::Prepared {
                sequence: 1,
                session_id: 9,
                display_name: "large.dsl".to_owned(),
                source_identity: SourceIdentity::from_bytes([4; 32]),
                index_identity: SourceIdentity::from_bytes([5; 32]),
                metadata: metadata(),
            },
            CaptureWorkerMessage::Window {
                sequence: 2,
                window: CaptureSampledWindow {
                    start_sample: 4_000_000_000,
                    end_sample: 4_500_000_000,
                    sample_step: 260_417,
                    channels: vec![CaptureSampledChannel {
                        channel: 7,
                        name: "D7".to_owned(),
                        initial: false,
                        transitions: Vec::new(),
                        waveform: Vec::new(),
                    }],
                },
            },
            CaptureWorkerMessage::Replay {
                sequence: 3,
                block: 500,
                blocks: vec![CaptureWorkerReplayBlock {
                    channel: 7,
                    block: 500,
                    start_sample: 8_388_608_000,
                    valid_samples: 16_777_216,
                    data: vec![0xaa, 0x55],
                }],
                next_channel: 2,
            },
            CaptureWorkerMessage::Failed {
                sequence: 3,
                message: "controlled failure".to_owned(),
            },
            CaptureWorkerMessage::Cancelled { sequence: 4 },
        ];

        for message in messages {
            let encoded = serde_json::to_vec(&message).unwrap();
            assert_eq!(
                serde_json::from_slice::<CaptureWorkerMessage>(&encoded).unwrap(),
                message
            );
        }
    }

    #[test]
    fn framed_batches_keep_replay_payloads_binary() {
        let payload = vec![0xa5; 1024 * 1024];
        let messages = vec![
            CaptureWorkerMessage::Progress {
                sequence: 7,
                progress: CaptureIndexBuildProgress {
                    completed: 1,
                    total: 2,
                },
            },
            CaptureWorkerMessage::Replay {
                sequence: 8,
                block: 12,
                blocks: vec![CaptureWorkerReplayBlock {
                    channel: 3,
                    block: 12,
                    start_sample: 1_200,
                    valid_samples: 100,
                    data: payload.clone(),
                }],
                next_channel: 1,
            },
        ];

        let encoded = encode_capture_worker_messages(&messages).unwrap();

        assert!(encoded.len() < payload.len() + 512);
        assert_eq!(decode_capture_worker_messages(&encoded).unwrap(), messages);
    }
}
