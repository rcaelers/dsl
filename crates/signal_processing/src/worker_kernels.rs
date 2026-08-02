use std::sync::Arc;

#[cfg(test)]
use super::capture_index_kernel::CaptureIndexBlockLevels;
use super::capture_index_kernel::{
    CaptureIndexBlockRequest, CaptureIndexBlockResult, build_capture_index_block,
};
use super::derived_word_store::{EncodeWordBlockRequest, encode_owned_word_block};
use super::events::{Word, WordPayload};
use super::work_executor::WorkerKernelRegistry;

const ENCODE_WORD_BLOCK_OPERATION: &str = "signal-processing.encode-word-block/v1";
const BUILD_CAPTURE_INDEX_BLOCK_OPERATION: &str = "signal-processing.build-capture-index-block/v1";
#[cfg(test)]
const L1_WORDS: usize = 4096;
#[cfg(test)]
const L2_WORDS: usize = 64;

/// Creates the catalog of finite, platform-neutral signal-processing kernels.
///
/// Hosts use the stable operation identifiers and opaque payloads in this
/// catalog. They do not need to understand either storage format.
pub fn portable_worker_kernels() -> WorkerKernelRegistry {
    let mut registry = WorkerKernelRegistry::new();
    registry
        .register(ENCODE_WORD_BLOCK_OPERATION, |payload| {
            let request = decode_word_block_request(&payload)?;
            encode_owned_word_block(request, Vec::new())
                .map(|result| result.bytes)
                .map_err(|error| error.to_string())
        })
        .expect("the derived-word operation identifier is unique and valid");
    registry
        .register(BUILD_CAPTURE_INDEX_BLOCK_OPERATION, |payload| {
            let request = decode_capture_index_request(&payload)?;
            build_capture_index_block(request).map(encode_capture_index_result)
        })
        .expect("the capture-index operation identifier is unique and valid");
    registry
}

#[cfg(test)]
fn encode_word_block_request(request: &EncodeWordBlockRequest) -> Vec<u8> {
    let mut payload = Vec::new();
    put_u64(&mut payload, request.sequence);
    put_u64(&mut payload, request.max_words);
    put_u64(&mut payload, request.restart_interval);
    put_u64(&mut payload, request.max_payload_bytes);
    put_u64(&mut payload, request.max_inter_word_gap_ns);
    put_u64(&mut payload, request.max_timestamp_span_ns);
    put_u64(&mut payload, request.words.len() as u64);
    for word in &request.words {
        put_u64(&mut payload, word.value);
        put_u64(&mut payload, word.timestamp_ns);
        put_u64(&mut payload, word.duration_ns);
        match &word.payload {
            None => payload.push(0),
            Some(WordPayload::Bytes(bytes)) => {
                payload.push(1);
                put_bytes(&mut payload, bytes);
            }
            Some(WordPayload::Text(text)) => {
                payload.push(2);
                put_bytes(&mut payload, text.as_bytes());
            }
        }
    }
    payload
}

fn decode_word_block_request(payload: &[u8]) -> Result<EncodeWordBlockRequest, String> {
    let mut reader = PayloadReader::new(payload);
    let sequence = reader.u64()?;
    let max_words = reader.u64()?;
    let restart_interval = reader.u64()?;
    let max_payload_bytes = reader.u64()?;
    let max_inter_word_gap_ns = reader.u64()?;
    let max_timestamp_span_ns = reader.u64()?;
    let word_count = reader.length("word count")?;
    let mut words = Vec::with_capacity(word_count);
    for _ in 0..word_count {
        let value = reader.u64()?;
        let timestamp_ns = reader.u64()?;
        let duration_ns = reader.u64()?;
        let word_payload = match reader.u8()? {
            0 => None,
            1 => Some(WordPayload::Bytes(Arc::from(reader.bytes()?.to_vec()))),
            2 => {
                let text = std::str::from_utf8(reader.bytes()?)
                    .map_err(|_| "worker word payload contains invalid UTF-8".to_string())?;
                Some(WordPayload::Text(Arc::from(text)))
            }
            _ => return Err("worker word payload has an unknown tag".to_string()),
        };
        words.push(Word {
            value,
            payload: word_payload,
            timestamp_ns,
            duration_ns,
        });
    }
    reader.finish()?;
    Ok(EncodeWordBlockRequest {
        sequence,
        max_words,
        restart_interval,
        max_payload_bytes,
        max_inter_word_gap_ns,
        max_timestamp_span_ns,
        words,
    })
}

#[cfg(test)]
fn encode_capture_index_request(request: &CaptureIndexBlockRequest) -> Vec<u8> {
    let mut payload = Vec::with_capacity(40 + request.packed_samples.len());
    put_u64(&mut payload, request.sequence);
    put_u64(&mut payload, request.channel);
    put_u64(&mut payload, request.block);
    put_u64(&mut payload, request.valid_samples);
    put_bytes(&mut payload, &request.packed_samples);
    payload
}

fn decode_capture_index_request(payload: &[u8]) -> Result<CaptureIndexBlockRequest, String> {
    let mut reader = PayloadReader::new(payload);
    let request = CaptureIndexBlockRequest {
        sequence: reader.u64()?,
        channel: reader.u64()?,
        block: reader.u64()?,
        valid_samples: reader.u64()?,
        packed_samples: reader.bytes()?.to_vec(),
    };
    reader.finish()?;
    Ok(request)
}

fn encode_capture_index_result(result: CaptureIndexBlockResult) -> Vec<u8> {
    let mut payload = Vec::new();
    put_u64(&mut payload, result.sequence);
    put_u64(&mut payload, result.channel);
    put_u64(&mut payload, result.block);
    put_u32(&mut payload, result.valid_samples);
    payload.push(u8::from(result.first));
    payload.push(u8::from(result.last));
    payload.push(u8::from(result.levels.is_some()));
    if let Some(levels) = result.levels {
        for word in levels
            .l1_toggle
            .into_iter()
            .chain(levels.l1_last)
            .chain(levels.l2_toggle)
            .chain(levels.l2_last)
        {
            put_u64(&mut payload, word);
        }
        put_u64(&mut payload, levels.l3_toggle);
        put_u64(&mut payload, levels.l3_last);
    }
    payload
}

#[cfg(test)]
fn decode_capture_index_result(payload: &[u8]) -> Result<CaptureIndexBlockResult, String> {
    let mut reader = PayloadReader::new(payload);
    let sequence = reader.u64()?;
    let channel = reader.u64()?;
    let block = reader.u64()?;
    let valid_samples = reader.u32()?;
    let first = reader.boolean()?;
    let last = reader.boolean()?;
    let levels = if reader.boolean()? {
        Some(CaptureIndexBlockLevels {
            l1_toggle: reader.u64_vec(L1_WORDS)?,
            l1_last: reader.u64_vec(L1_WORDS)?,
            l2_toggle: reader.u64_vec(L2_WORDS)?,
            l2_last: reader.u64_vec(L2_WORDS)?,
            l3_toggle: reader.u64()?,
            l3_last: reader.u64()?,
        })
    } else {
        None
    };
    reader.finish()?;
    Ok(CaptureIndexBlockResult {
        sequence,
        channel,
        block,
        valid_samples,
        first,
        last,
        levels,
    })
}

fn put_u32(payload: &mut Vec<u8>, value: u32) {
    payload.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(payload: &mut Vec<u8>, value: u64) {
    payload.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
fn put_bytes(payload: &mut Vec<u8>, bytes: &[u8]) {
    put_u64(payload, bytes.len() as u64);
    payload.extend_from_slice(bytes);
}

struct PayloadReader<'a> {
    payload: &'a [u8],
    cursor: usize,
}

impl<'a> PayloadReader<'a> {
    fn new(payload: &'a [u8]) -> Self {
        Self { payload, cursor: 0 }
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(*self.take(1)?.first().unwrap())
    }

    #[cfg(test)]
    fn boolean(&mut self) -> Result<bool, String> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("worker payload contains an invalid boolean".to_string()),
        }
    }

    #[cfg(test)]
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn length(&mut self, what: &str) -> Result<usize, String> {
        usize::try_from(self.u64()?)
            .map_err(|_| format!("worker {what} exceeds this address space"))
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let length = self.length("byte payload length")?;
        self.take(length)
    }

    #[cfg(test)]
    fn u64_vec(&mut self, length: usize) -> Result<Vec<u64>, String> {
        (0..length).map(|_| self.u64()).collect()
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or_else(|| "worker payload length overflow".to_string())?;
        let bytes = self
            .payload
            .get(self.cursor..end)
            .ok_or_else(|| "worker payload is truncated".to_string())?;
        self.cursor = end;
        Ok(bytes)
    }

    fn finish(self) -> Result<(), String> {
        if self.cursor == self.payload.len() {
            Ok(())
        } else {
            Err("worker payload contains trailing bytes".to_string())
        }
    }
}

#[cfg(test)]
mod worker_kernel_tests {
    use super::*;
    use crate::{WorkerMessage, WorkerOperation, WorkerRequest};

    #[test]
    fn derived_word_kernel_round_trips_binary_and_text_payloads() {
        let request = EncodeWordBlockRequest {
            sequence: 4,
            max_words: 16,
            restart_interval: 4,
            max_payload_bytes: 1024,
            max_inter_word_gap_ns: u64::MAX,
            max_timestamp_span_ns: u64::MAX,
            words: vec![
                Word::bytes_with_tag(0x12, [0xaa, 0x55], 10, 2),
                Word::text("ready", 20, 3),
            ],
        };
        let decoded = decode_word_block_request(&encode_word_block_request(&request)).unwrap();
        assert_eq!(decoded, request);

        let registry = portable_worker_kernels();
        let expected = encode_owned_word_block(request.clone(), Vec::new())
            .unwrap()
            .bytes;
        let message = registry.execute(WorkerRequest {
            sequence: 77,
            operation: WorkerOperation::new(ENCODE_WORD_BLOCK_OPERATION).unwrap(),
            payload: encode_word_block_request(&request),
        });
        let WorkerMessage::Complete { sequence, payload } = message else {
            panic!("derived-word kernel did not complete");
        };
        assert_eq!(sequence, 77);
        assert_eq!(payload, expected);
    }

    #[test]
    fn capture_index_kernel_round_trips_owned_input_and_output() {
        let above_wasm32 = u64::from(u32::MAX) + 71;
        let request = CaptureIndexBlockRequest {
            sequence: above_wasm32,
            channel: above_wasm32 + 1,
            block: above_wasm32 + 2,
            valid_samples: 16,
            packed_samples: vec![0b1111_0000, 0b1010_1010],
        };
        assert_eq!(
            decode_capture_index_request(&encode_capture_index_request(&request)).unwrap(),
            request
        );

        let registry = portable_worker_kernels();
        let expected = build_capture_index_block(request.clone()).unwrap();
        let message = registry.execute(WorkerRequest {
            sequence: 91,
            operation: WorkerOperation::new(BUILD_CAPTURE_INDEX_BLOCK_OPERATION).unwrap(),
            payload: encode_capture_index_request(&request),
        });
        let WorkerMessage::Complete { sequence, payload } = message else {
            panic!("capture-index kernel did not complete");
        };
        assert_eq!(sequence, 91);
        let result = decode_capture_index_result(&payload).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn malformed_kernel_payload_is_reported_as_a_failed_message() {
        let registry = portable_worker_kernels();
        let message = registry.execute(WorkerRequest {
            sequence: 12,
            operation: WorkerOperation::new(BUILD_CAPTURE_INDEX_BLOCK_OPERATION).unwrap(),
            payload: vec![1, 2, 3],
        });
        assert!(matches!(
            message,
            WorkerMessage::Failed { sequence: 12, .. }
        ));
    }
}
