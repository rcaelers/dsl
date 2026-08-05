use signal_runtime::WorkerKernelRegistry;

const LEVEL_POWER: usize = 6;
const L1_WORDS: usize = 1 << (LEVEL_POWER * 2);
const L2_WORDS: usize = 1 << LEVEL_POWER;
const BUILD_CAPTURE_INDEX_BLOCK_OPERATION: &str = "signal-processing.build-capture-index-block/v1";

/// Adds the finite capture-index operation to a portable worker registry.
pub fn register_capture_worker_kernel(registry: &mut WorkerKernelRegistry) {
    registry
        .register(BUILD_CAPTURE_INDEX_BLOCK_OPERATION, |payload| {
            let request = decode_capture_index_request(&payload)?;
            build_capture_index_block(request).map(encode_capture_index_result)
        })
        .expect("the capture-index operation identifier is unique and valid");
}

/// Owned input for one finite capture-index operation.
///
/// Source access stays with the host. The kernel receives only the packed
/// samples and fixed-width coordinates needed to build one index leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureIndexBlockRequest {
    pub(crate) sequence: u64,
    pub(crate) channel: u64,
    pub(crate) block: u64,
    pub(crate) valid_samples: u64,
    pub(crate) packed_samples: Vec<u8>,
}

/// Transferable hierarchy data for a non-constant capture-index leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureIndexBlockLevels {
    pub(crate) l1_toggle: Vec<u64>,
    pub(crate) l1_last: Vec<u64>,
    pub(crate) l2_toggle: Vec<u64>,
    pub(crate) l2_last: Vec<u64>,
    pub(crate) l3_toggle: u64,
    pub(crate) l3_last: u64,
}

/// Owned output from one finite capture-index operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureIndexBlockResult {
    pub(crate) sequence: u64,
    pub(crate) channel: u64,
    pub(crate) block: u64,
    pub(crate) valid_samples: u32,
    pub(crate) first: bool,
    pub(crate) last: bool,
    pub(crate) levels: Option<CaptureIndexBlockLevels>,
}

pub(crate) fn build_capture_index_block(
    request: CaptureIndexBlockRequest,
) -> Result<CaptureIndexBlockResult, String> {
    build_capture_index_block_from_packed(
        request.sequence,
        request.channel,
        request.block,
        request.valid_samples,
        &request.packed_samples,
    )
}

pub(crate) fn build_capture_index_block_from_packed(
    sequence: u64,
    channel: u64,
    block: u64,
    requested_valid_samples: u64,
    packed_samples: &[u8],
) -> Result<CaptureIndexBlockResult, String> {
    let available_samples = (packed_samples.len() as u64).saturating_mul(8);
    if requested_valid_samples > available_samples {
        return Err(format!(
            "capture-index request declares {} valid samples but contains only {available_samples}",
            requested_valid_samples
        ));
    }
    let valid_samples = u32::try_from(requested_valid_samples)
        .map_err(|_| "capture-index leaf exceeds the fixed-width sample limit".to_string())?;
    if valid_samples == 0 {
        return Ok(CaptureIndexBlockResult {
            sequence,
            channel,
            block,
            valid_samples,
            first: false,
            last: false,
            levels: None,
        });
    }

    let first = packed_bit(packed_samples, 0);
    let last = packed_bit(packed_samples, valid_samples as usize - 1);
    let mut entering = first;
    let mut levels = CaptureIndexBlockLevels {
        l1_toggle: vec![0; L1_WORDS],
        l1_last: vec![0; L1_WORDS],
        l2_toggle: vec![0; L2_WORDS],
        l2_last: vec![0; L2_WORDS],
        l3_toggle: 0,
        l3_last: 0,
    };

    let l1_groups = (valid_samples as usize).div_ceil(64);
    let full_l1_groups = valid_samples as usize / 64;
    let (full_l1_chunks, _) = packed_samples[..full_l1_groups * 8].as_chunks::<8>();
    for (group, chunk) in full_l1_chunks.iter().enumerate() {
        record_l1_group(
            &mut levels.l1_toggle,
            &mut levels.l1_last,
            group,
            u64::from_le_bytes(*chunk),
            64,
            &mut entering,
        );
    }
    if full_l1_groups < l1_groups {
        let (word, valid_bits) =
            partial_l1_word(packed_samples, full_l1_groups, valid_samples as usize);
        record_l1_group(
            &mut levels.l1_toggle,
            &mut levels.l1_last,
            full_l1_groups,
            word,
            valid_bits,
            &mut entering,
        );
    }

    let l2_groups = l1_groups.div_ceil(64);
    for group in 0..l2_groups {
        if levels.l1_toggle[group] != 0 {
            set_bit(&mut levels.l2_toggle[group / 64], group % 64);
        }
        let last_l1_group = ((group + 1) * 64).min(l1_groups).saturating_sub(1);
        if bit(levels.l1_last[last_l1_group / 64], last_l1_group % 64) {
            set_bit(&mut levels.l2_last[group / 64], group % 64);
        }
    }

    let l3_groups = l2_groups.div_ceil(64);
    for group in 0..l3_groups {
        if levels.l2_toggle[group] != 0 {
            set_bit(&mut levels.l3_toggle, group);
        }
        let last_l2_group = ((group + 1) * 64).min(l2_groups).saturating_sub(1);
        if bit(levels.l2_last[last_l2_group / 64], last_l2_group % 64) {
            set_bit(&mut levels.l3_last, group);
        }
    }

    Ok(CaptureIndexBlockResult {
        sequence,
        channel,
        block,
        valid_samples,
        first,
        last,
        levels: (levels.l3_toggle != 0).then_some(levels),
    })
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
    payload.extend_from_slice(&result.valid_samples.to_le_bytes());
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

fn put_u64(payload: &mut Vec<u8>, value: u64) {
    payload.extend_from_slice(&value.to_le_bytes());
}

struct PayloadReader<'a> {
    payload: &'a [u8],
    cursor: usize,
}

impl<'a> PayloadReader<'a> {
    fn new(payload: &'a [u8]) -> Self {
        Self { payload, cursor: 0 }
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let length = usize::try_from(self.u64()?)
            .map_err(|_| "worker byte payload length exceeds this address space".to_string())?;
        self.take(length)
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

fn record_l1_group(
    l1_toggle: &mut [u64],
    l1_last: &mut [u64],
    group: usize,
    word: u64,
    valid_bits: usize,
    entering: &mut bool,
) {
    let first_bit = (word & 1) != 0;
    if first_bit != *entering || l1_word_has_internal_toggle(word, valid_bits) {
        set_bit(&mut l1_toggle[group / 64], group % 64);
    }

    *entering = bit(word, valid_bits - 1);
    if *entering {
        set_bit(&mut l1_last[group / 64], group % 64);
    }
}

fn partial_l1_word(data: &[u8], group: usize, valid_samples: usize) -> (u64, usize) {
    let sample_start = group * 64;
    let valid_bits = (valid_samples - sample_start).min(64);
    let byte_start = group * 8;
    let mut bytes = [0_u8; 8];
    let available = data.len().saturating_sub(byte_start).min(8);
    if available > 0 {
        bytes[..available].copy_from_slice(&data[byte_start..byte_start + available]);
    }
    let mut word = u64::from_le_bytes(bytes);
    if valid_bits < 64 {
        word &= (1_u64 << valid_bits) - 1;
    }
    (word, valid_bits)
}

fn l1_word_has_internal_toggle(word: u64, valid_bits: usize) -> bool {
    if valid_bits <= 1 {
        return false;
    }
    let valid_mask = if valid_bits == 64 {
        u64::MAX
    } else {
        (1_u64 << valid_bits) - 1
    };
    let internal_mask = valid_mask & !1_u64;
    (word ^ (word << 1)) & internal_mask != 0
}

fn packed_bit(data: &[u8], index: usize) -> bool {
    data.get(index / 8)
        .is_some_and(|byte| byte & (1 << (index % 8)) != 0)
}

fn bit(word: u64, index: usize) -> bool {
    index < 64 && ((word >> index) & 1) != 0
}

fn set_bit(word: &mut u64, index: usize) {
    if index < 64 {
        *word |= 1_u64 << index;
    }
}

#[cfg(test)]
mod capture_index_kernel_tests {
    use super::{CaptureIndexBlockRequest, build_capture_index_block, l1_word_has_internal_toggle};

    #[test]
    fn owned_request_builds_a_result_without_host_state() {
        let request = CaptureIndexBlockRequest {
            sequence: 9,
            channel: 3,
            block: 7,
            valid_samples: 16,
            packed_samples: vec![0b1111_0000, 0b1010_1010],
        };
        let result = build_capture_index_block(request).unwrap();

        assert_eq!(result.sequence, 9);
        assert_eq!(result.channel, 3);
        assert_eq!(result.block, 7);
        assert_eq!(result.valid_samples, 16);
        assert!(result.levels.is_some());
    }

    #[test]
    fn rejects_sample_counts_larger_than_the_owned_payload() {
        let error = build_capture_index_block(CaptureIndexBlockRequest {
            sequence: 0,
            channel: 0,
            block: 0,
            valid_samples: 9,
            packed_samples: vec![0],
        })
        .unwrap_err();

        assert!(error.contains("contains only 8"));
    }

    #[test]
    fn word_toggle_detection_handles_boundaries_and_partial_groups() {
        assert!(!l1_word_has_internal_toggle(0, 64));
        assert!(!l1_word_has_internal_toggle(u64::MAX, 64));
        assert!(l1_word_has_internal_toggle(0b10, 2));
        assert!(!l1_word_has_internal_toggle(0b10, 1));
    }
}
