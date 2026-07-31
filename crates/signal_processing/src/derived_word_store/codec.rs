use super::config::BlockCodecConfig;
use super::errors::{CodecError, CodecResult};
use super::format::{
    BLOCK_CHECKSUM_OFFSET, BLOCK_FLAG_GROUPED_TIMESTAMPS, BLOCK_FLAG_HAS_DURATIONS,
    BLOCK_FLAG_HAS_PAYLOADS, BLOCK_HEADER_SIZE, DEFAULT_MAX_WORDS_PER_BLOCK, RESTART_ENTRY_SIZE,
    RestartEntry, WordBlockHeader,
};
use super::vlq::{decode_u64, encode_u64, encoded_len};
use crate::crc32c::block_checksum;
use crate::events::{Word, WordPayload};

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PushResult {
    Appended,
    BlockFull,
}

/// Accumulates one ordered block and predicts configured block boundaries.
#[derive(Debug)]
pub(crate) struct WordBlockBuilder {
    config: BlockCodecConfig,
    words: Vec<Word>,
    timestamp_bytes: usize,
    duration_bytes: usize,
    duration_count: usize,
    last_duration_index: usize,
    payload_entry_bytes: usize,
    payload_count: usize,
    last_payload_index: usize,
    max_value: u64,
}

impl WordBlockBuilder {
    pub(crate) fn new(config: BlockCodecConfig) -> CodecResult<Self> {
        if config.restart_interval == 0 {
            return Err(CodecError::InvalidRestartInterval);
        }
        if config.max_words == 0 {
            return Err(CodecError::InvalidConfiguration(
                "max_words must be greater than zero",
            ));
        }
        if config.max_words > u32::MAX as usize {
            return Err(CodecError::InvalidConfiguration(
                "max_words must fit in u32",
            ));
        }
        if config.max_payload_bytes == 0 {
            return Err(CodecError::InvalidConfiguration(
                "max_payload_bytes must be greater than zero",
            ));
        }
        Ok(Self {
            config,
            words: Vec::with_capacity(config.max_words.min(DEFAULT_MAX_WORDS_PER_BLOCK)),
            timestamp_bytes: 0,
            duration_bytes: 0,
            duration_count: 0,
            last_duration_index: 0,
            payload_entry_bytes: 0,
            payload_count: 0,
            last_payload_index: 0,
            max_value: 0,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }

    pub(crate) fn is_at_word_limit(&self) -> bool {
        self.words.len() >= self.config.max_words
    }

    pub(crate) fn is_duration_free(&self) -> bool {
        self.duration_count == 0
    }

    pub(crate) fn words(&self) -> &[Word] {
        &self.words
    }

    /// Appends `word`, or reports that the current non-empty block should be
    /// committed first. `word` is not consumed when `BlockFull` is returned.
    #[cfg(test)]
    fn push(&mut self, word: Word) -> CodecResult<PushResult> {
        self.validate_order(&word)?;
        self.push_ordered(word)
    }

    /// Appends a word whose global ordering was already checked by the
    /// owning stream writer. This avoids validating every word twice on the
    /// high-volume live-index path while retaining all block-size checks.
    #[cfg(test)]
    fn push_ordered(&mut self, word: Word) -> CodecResult<PushResult> {
        if !self.words.is_empty() && self.would_close_before(&word) {
            return Ok(PushResult::BlockFull);
        }
        if self.words.len() == u32::MAX as usize {
            return Err(CodecError::TooManyWords(self.words.len() + 1));
        }
        self.append(word);
        Ok(PushResult::Appended)
    }

    /// Extends this block with the largest prefix that fits. The caller owns
    /// global timestamp validation. Sizing is accumulated in locals and the
    /// accepted words are copied into the builder once, avoiding billions of
    /// individual `Vec::push` and repeated projection calls for dense lanes.
    pub(crate) fn extend_ordered(&mut self, words: &[Word]) -> usize {
        let available = self.config.max_words.saturating_sub(self.words.len());
        let candidates = &words[..words.len().min(available)];
        if self.duration_count == 0
            && self.payload_count == 0
            && self.duration_free_payload_fits_at_max_words()
            && candidates
                .iter()
                .all(|word| word.duration_ns == 0 && word.is_numeric())
        {
            return self.extend_duration_free(candidates);
        }

        let original_len = self.words.len();
        let mut accepted = 0usize;
        let mut timestamp_bytes = self.timestamp_bytes;
        let mut duration_bytes = self.duration_bytes;
        let mut duration_count = self.duration_count;
        let mut last_duration_index = self.last_duration_index;
        let mut payload_entry_bytes = self.payload_entry_bytes;
        let mut payload_count = self.payload_count;
        let mut last_payload_index = self.last_payload_index;
        let mut max_value = self.max_value;
        let mut previous_timestamp = self.words.last().map(|word| word.timestamp_ns);
        let first_timestamp = self
            .words
            .first()
            .map(|word| word.timestamp_ns)
            .or_else(|| words.first().map(|word| word.timestamp_ns));

        for word in words {
            let next_index = original_len + accepted;
            if let Some(previous_timestamp) = previous_timestamp {
                if next_index >= self.config.max_words
                    || word.timestamp_ns.saturating_sub(previous_timestamp)
                        > self.config.max_inter_word_gap_ns
                    || word
                        .timestamp_ns
                        .saturating_sub(first_timestamp.expect("non-empty block prefix"))
                        > self.config.max_timestamp_span_ns
                {
                    break;
                }

                let next_timestamp_bytes = timestamp_bytes
                    + encoded_len(word.timestamp_ns.saturating_sub(previous_timestamp));
                let next_max_value = max_value.max(word.value);
                let next_value_bytes = value_width(next_max_value);
                let (next_duration_bytes, next_duration_count, next_last_duration_index) =
                    if word.duration_ns == 0 {
                        (duration_bytes, duration_count, last_duration_index)
                    } else {
                        let index_delta = if duration_count == 0 {
                            next_index
                        } else {
                            next_index - last_duration_index
                        };
                        (
                            duration_bytes
                                + encoded_len(index_delta as u64)
                                + encoded_len(word.duration_ns),
                            duration_count + 1,
                            next_index,
                        )
                    };
                let word_count = next_index + 1;
                let (next_payload_entry_bytes, next_payload_count, next_last_payload_index) =
                    if let Some(payload) = &word.payload {
                        let index_delta = if payload_count == 0 {
                            next_index
                        } else {
                            next_index - last_payload_index
                        };
                        (
                            payload_entry_bytes
                                + encoded_len(index_delta as u64)
                                + 1
                                + encoded_len(payload_len(payload) as u64)
                                + payload_len(payload),
                            payload_count + 1,
                            next_index,
                        )
                    } else {
                        (payload_entry_bytes, payload_count, last_payload_index)
                    };
                let record_bytes = next_timestamp_bytes + word_count * next_value_bytes;
                let restart_count = word_count.div_ceil(self.config.restart_interval);
                let payload_bytes = if next_payload_count > 0 {
                    encoded_len(next_payload_count as u64) + next_payload_entry_bytes
                } else {
                    0
                };
                if record_bytes
                    + restart_count * RESTART_ENTRY_SIZE
                    + next_duration_bytes
                    + payload_bytes
                    > self.config.max_payload_bytes
                {
                    break;
                }
                timestamp_bytes = next_timestamp_bytes;
                max_value = next_max_value;
                duration_bytes = next_duration_bytes;
                duration_count = next_duration_count;
                last_duration_index = next_last_duration_index;
                payload_entry_bytes = next_payload_entry_bytes;
                payload_count = next_payload_count;
                last_payload_index = next_last_payload_index;
            } else {
                timestamp_bytes += encoded_len(0);
                if word.duration_ns != 0 {
                    duration_bytes += encoded_len(0) + encoded_len(word.duration_ns);
                    duration_count = 1;
                    last_duration_index = 0;
                }
                max_value = word.value;
                if let Some(payload) = &word.payload {
                    payload_entry_bytes = encoded_len(0)
                        + 1
                        + encoded_len(payload_len(payload) as u64)
                        + payload_len(payload);
                    payload_count = 1;
                    last_payload_index = 0;
                }
            }
            previous_timestamp = Some(word.timestamp_ns);
            accepted += 1;
        }

        self.timestamp_bytes = timestamp_bytes;
        self.duration_bytes = duration_bytes;
        self.duration_count = duration_count;
        self.last_duration_index = last_duration_index;
        self.payload_entry_bytes = payload_entry_bytes;
        self.payload_count = payload_count;
        self.last_payload_index = last_payload_index;
        self.max_value = max_value;
        self.words.extend_from_slice(&words[..accepted]);
        accepted
    }

    /// With no duration table, the worst possible timestamp/value record is
    /// bounded (10-byte VLQ + 8-byte value). When that worst case fits at the
    /// configured word limit, payload sizing cannot close the block and the
    /// dense path only needs timestamp-boundary checks.
    fn duration_free_payload_fits_at_max_words(&self) -> bool {
        let max_words = self.config.max_words;
        let restart_bytes = max_words
            .div_ceil(self.config.restart_interval)
            .saturating_mul(RESTART_ENTRY_SIZE);
        max_words
            .saturating_mul(10 + size_of::<u64>())
            .saturating_add(restart_bytes)
            <= self.config.max_payload_bytes
    }

    fn extend_duration_free(&mut self, words: &[Word]) -> usize {
        let first_timestamp = self
            .words
            .first()
            .map(|word| word.timestamp_ns)
            .or_else(|| words.first().map(|word| word.timestamp_ns));
        let mut previous_timestamp = self.words.last().map(|word| word.timestamp_ns);
        let mut accepted = 0usize;
        let mut timestamp_bytes = self.timestamp_bytes;
        let mut max_value = self.max_value;
        for word in words {
            if let Some(previous_timestamp) = previous_timestamp {
                let delta = word.timestamp_ns.saturating_sub(previous_timestamp);
                if delta > self.config.max_inter_word_gap_ns
                    || word
                        .timestamp_ns
                        .saturating_sub(first_timestamp.expect("non-empty block prefix"))
                        > self.config.max_timestamp_span_ns
                {
                    break;
                }
                timestamp_bytes += if delta <= 0x7f { 1 } else { encoded_len(delta) };
            } else {
                timestamp_bytes += encoded_len(0);
            }
            previous_timestamp = Some(word.timestamp_ns);
            max_value = max_value.max(word.value);
            accepted += 1;
        }
        self.timestamp_bytes = timestamp_bytes;
        self.max_value = max_value;
        self.words.extend_from_slice(&words[..accepted]);
        accepted
    }

    pub(crate) fn clear(&mut self) {
        self.words.clear();
        self.reset_metadata();
    }

    pub(crate) fn empty_like(&self) -> Self {
        Self::new(self.config).expect("an existing builder has valid configuration")
    }

    fn reset_metadata(&mut self) {
        self.timestamp_bytes = 0;
        self.duration_bytes = 0;
        self.duration_count = 0;
        self.last_duration_index = 0;
        self.payload_entry_bytes = 0;
        self.payload_count = 0;
        self.last_payload_index = 0;
        self.max_value = 0;
    }

    pub(crate) fn encode(
        &self,
        sequence: u64,
        output: &mut Vec<u8>,
    ) -> CodecResult<EncodedBlockMetadata> {
        encode_validated_word_block_with_interval(
            sequence,
            &self.words,
            self.config.restart_interval,
            value_width(self.max_value),
            self.timestamp_bytes,
            self.duration_count == 0
                && self.payload_count == 0
                && self.max_value <= u8::MAX.into()
                && self.timestamp_bytes == self.words.len(),
            output,
        )
    }

    #[cfg(test)]
    fn validate_order(&self, word: &Word) -> CodecResult<()> {
        if let Some(previous) = self.words.last()
            && word.timestamp_ns < previous.timestamp_ns
        {
            return Err(CodecError::OutOfOrder {
                index: self.words.len(),
                previous_timestamp_ns: previous.timestamp_ns,
                timestamp_ns: word.timestamp_ns,
            });
        }
        Ok(())
    }

    #[cfg(test)]
    fn would_close_before(&self, word: &Word) -> bool {
        let first = self.words.first().expect("non-empty builder");
        let last = self.words.last().expect("non-empty builder");
        if self.words.len() >= self.config.max_words
            || word.timestamp_ns - last.timestamp_ns > self.config.max_inter_word_gap_ns
            || word.timestamp_ns - first.timestamp_ns > self.config.max_timestamp_span_ns
        {
            return true;
        }

        let next_index = self.words.len();
        let timestamp_bytes =
            self.timestamp_bytes + encoded_len(word.timestamp_ns.saturating_sub(last.timestamp_ns));
        let value_bytes = value_width(self.max_value.max(word.value));
        let duration_bytes = self.duration_bytes
            + if word.duration_ns == 0 {
                0
            } else {
                let index_delta = if self.duration_count == 0 {
                    next_index
                } else {
                    next_index - self.last_duration_index
                };
                encoded_len(index_delta as u64) + encoded_len(word.duration_ns)
            };
        let record_bytes = timestamp_bytes + (next_index + 1) * value_bytes;
        let restart_count = (next_index + 1).div_ceil(self.config.restart_interval);
        let (payload_entry_bytes, payload_count) = if let Some(payload) = &word.payload {
            let index_delta = if self.payload_count == 0 {
                next_index
            } else {
                next_index - self.last_payload_index
            };
            (
                self.payload_entry_bytes
                    + encoded_len(index_delta as u64)
                    + 1
                    + encoded_len(payload_len(payload) as u64)
                    + payload_len(payload),
                self.payload_count + 1,
            )
        } else {
            (self.payload_entry_bytes, self.payload_count)
        };
        let payload_bytes = if payload_count > 0 {
            encoded_len(payload_count as u64) + payload_entry_bytes
        } else {
            0
        };
        record_bytes + restart_count * RESTART_ENTRY_SIZE + duration_bytes + payload_bytes
            > self.config.max_payload_bytes
    }

    #[cfg(test)]
    fn append(&mut self, word: Word) {
        let index = self.words.len();
        let delta = self
            .words
            .last()
            .map_or(0, |previous| word.timestamp_ns - previous.timestamp_ns);
        self.timestamp_bytes += encoded_len(delta);
        if word.duration_ns != 0 {
            let index_delta = if self.duration_count == 0 {
                index
            } else {
                index - self.last_duration_index
            };
            self.duration_bytes += encoded_len(index_delta as u64) + encoded_len(word.duration_ns);
            self.duration_count += 1;
            self.last_duration_index = index;
        }
        if let Some(payload) = &word.payload {
            let index_delta = if self.payload_count == 0 {
                index
            } else {
                index - self.last_payload_index
            };
            self.payload_entry_bytes += encoded_len(index_delta as u64)
                + 1
                + encoded_len(payload_len(payload) as u64)
                + payload_len(payload);
            self.payload_count += 1;
            self.last_payload_index = index;
        }
        self.max_value = self.max_value.max(word.value);
        self.words.push(word);
    }
}

impl Default for WordBlockBuilder {
    fn default() -> Self {
        Self::new(BlockCodecConfig::default()).expect("default block codec configuration is valid")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedBlockMetadata {
    pub(crate) header: WordBlockHeader,
    pub(crate) restarts: Vec<RestartEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedWordBlock {
    pub header: WordBlockHeader,
    pub restarts: Vec<RestartEntry>,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedWordRange {
    pub(crate) header: WordBlockHeader,
    pub(crate) words: Vec<Word>,
    pub(crate) complete: bool,
    pub(crate) decoded_records: usize,
}

#[cfg(test)]
fn encode_word_block_with_interval(
    sequence: u64,
    words: &[Word],
    restart_interval: usize,
    output: &mut Vec<u8>,
) -> CodecResult<EncodedBlockMetadata> {
    if words.is_empty() {
        return Err(CodecError::EmptyBlock);
    }
    if restart_interval == 0 {
        return Err(CodecError::InvalidRestartInterval);
    }
    if words.len() > u32::MAX as usize {
        return Err(CodecError::TooManyWords(words.len()));
    }
    validate_order(words)?;

    let value_bytes = value_width(words.iter().map(|word| word.value).max().unwrap());
    let timestamp_bytes = words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            encoded_len(if index == 0 {
                0
            } else {
                word.timestamp_ns - words[index - 1].timestamp_ns
            })
        })
        .sum();
    let dense_byte_words = value_bytes == 1
        && words
            .iter()
            .all(|word| word.duration_ns == 0 && word.payload.is_none())
        && words
            .windows(2)
            .all(|pair| pair[1].timestamp_ns - pair[0].timestamp_ns <= 0x7f);
    encode_validated_word_block_with_interval(
        sequence,
        words,
        restart_interval,
        value_bytes,
        timestamp_bytes,
        dense_byte_words,
        output,
    )
}

fn encode_validated_word_block_with_interval(
    sequence: u64,
    words: &[Word],
    restart_interval: usize,
    value_bytes: usize,
    timestamp_bytes: usize,
    dense_byte_words: bool,
    output: &mut Vec<u8>,
) -> CodecResult<EncodedBlockMetadata> {
    debug_assert!(!words.is_empty());
    debug_assert!(restart_interval > 0);
    let mut records = Vec::with_capacity(words.len() * (value_bytes + 1));
    let mut durations = Vec::new();
    let mut payloads = Vec::new();
    let mut restarts = Vec::with_capacity(words.len().div_ceil(restart_interval));
    let mut previous_timestamp = words[0].timestamp_ns;
    let mut previous_duration_index = 0usize;
    let mut duration_count = 0usize;
    let mut previous_payload_index = 0usize;
    let mut emitted_payload_count = 0usize;
    let payload_count = words.iter().filter(|word| word.payload.is_some()).count();
    if payload_count > 0 {
        encode_u64(payload_count as u64, &mut payloads);
    }

    let grouped_records = if dense_byte_words {
        encode_grouped_dense_records(words, restart_interval, timestamp_bytes, value_bytes)
    } else {
        None
    };
    let grouped_timestamps = grouped_records.is_some();
    if let Some((grouped, grouped_restarts)) = grouped_records {
        records = grouped;
        restarts = grouped_restarts;
    } else if dense_byte_words {
        debug_assert_eq!(value_bytes, 1);
        for (index, word) in words.iter().enumerate() {
            if index.is_multiple_of(restart_interval) {
                restarts.push(RestartEntry {
                    timestamp_ns: word.timestamp_ns,
                    payload_offset: (index * 2) as u32,
                    record_index: index as u32,
                });
            }
            let delta = if index == 0 {
                0
            } else {
                word.timestamp_ns - words[index - 1].timestamp_ns
            };
            debug_assert!(delta <= 0x7f);
            records.push(delta as u8);
            records.push(word.value as u8);
        }
    } else {
        for (index, word) in words.iter().enumerate() {
            if index.is_multiple_of(restart_interval) {
                restarts.push(RestartEntry {
                    timestamp_ns: word.timestamp_ns,
                    payload_offset: u32::try_from(records.len()).map_err(|_| {
                        CodecError::InvalidFormat("record payload exceeds 4 GiB".to_string())
                    })?,
                    record_index: index as u32,
                });
            }
            let delta = if index == 0 {
                0
            } else {
                word.timestamp_ns - previous_timestamp
            };
            encode_u64(delta, &mut records);
            append_value(word.value, value_bytes, &mut records);
            previous_timestamp = word.timestamp_ns;

            if word.duration_ns != 0 {
                let index_delta = if duration_count == 0 {
                    index
                } else {
                    index - previous_duration_index
                };
                encode_u64(index_delta as u64, &mut durations);
                encode_u64(word.duration_ns, &mut durations);
                previous_duration_index = index;
                duration_count += 1;
            }
            if let Some(payload) = &word.payload {
                let index_delta = if emitted_payload_count == 0 {
                    index
                } else {
                    index - previous_payload_index
                };
                encode_u64(index_delta as u64, &mut payloads);
                match payload {
                    WordPayload::Bytes(bytes) => {
                        payloads.push(1);
                        encode_u64(bytes.len() as u64, &mut payloads);
                        payloads.extend_from_slice(bytes);
                    }
                    WordPayload::Text(text) => {
                        payloads.push(2);
                        encode_u64(text.len() as u64, &mut payloads);
                        payloads.extend_from_slice(text.as_bytes());
                    }
                }
                previous_payload_index = index;
                emitted_payload_count += 1;
            }
        }
    }

    let restart_table_offset = BLOCK_HEADER_SIZE
        .checked_add(records.len())
        .ok_or_else(|| invalid("word-block size overflow"))?;
    let duration_table_offset = restart_table_offset
        .checked_add(restarts.len() * RESTART_ENTRY_SIZE)
        .ok_or_else(|| invalid("word-block size overflow"))?;
    let unpadded_len = duration_table_offset
        .checked_add(durations.len())
        .and_then(|length| length.checked_add(payloads.len()))
        .ok_or_else(|| invalid("word-block size overflow"))?;
    let block_len = unpadded_len
        .checked_add(7)
        .map(|length| length & !7)
        .ok_or_else(|| invalid("word-block size overflow"))?;

    let mut header = WordBlockHeader {
        flags: (if duration_count > 0 {
            BLOCK_FLAG_HAS_DURATIONS
        } else {
            0
        }) | (if payload_count > 0 {
            BLOCK_FLAG_HAS_PAYLOADS
        } else {
            0
        }) | (if grouped_timestamps {
            BLOCK_FLAG_GROUPED_TIMESTAMPS
        } else {
            0
        }),
        sequence,
        first_timestamp_ns: words[0].timestamp_ns,
        last_timestamp_ns: words.last().unwrap().timestamp_ns,
        word_count: words.len() as u32,
        value_bytes: value_bytes as u8,
        record_payload_len: to_u32(records.len(), "record payload")?,
        restart_count: to_u32(restarts.len(), "restart table")?,
        restart_table_offset: to_u32(restart_table_offset, "restart table offset")?,
        duration_count: to_u32(duration_count, "duration table")?,
        duration_table_offset: to_u32(duration_table_offset, "duration table offset")?,
        block_len: to_u32(block_len, "word block")?,
        crc32c: 0,
    };

    output.clear();
    output.resize(BLOCK_HEADER_SIZE, 0);
    output.extend_from_slice(&records);
    for restart in &restarts {
        restart.append_to(output);
    }
    output.extend_from_slice(&durations);
    output.extend_from_slice(&payloads);
    output.resize(block_len, 0);
    header.write_to(output);
    header.crc32c = block_checksum(output, BLOCK_CHECKSUM_OFFSET);
    header.write_to(output);

    Ok(EncodedBlockMetadata { header, restarts })
}

fn encode_grouped_dense_records(
    words: &[Word],
    restart_interval: usize,
    timestamp_bytes: usize,
    value_bytes: usize,
) -> Option<(Vec<u8>, Vec<RestartEntry>)> {
    debug_assert_eq!(value_bytes, 1);
    let legacy_len = timestamp_bytes.saturating_add(words.len());
    let mut records = Vec::with_capacity(words.len().saturating_add(words.len() / 32));
    let mut restarts = Vec::with_capacity(words.len().div_ceil(restart_interval));

    for (group_index, group) in words.chunks(restart_interval).enumerate() {
        let record_index = group_index * restart_interval;
        restarts.push(RestartEntry {
            timestamp_ns: group[0].timestamp_ns,
            payload_offset: u32::try_from(records.len()).ok()?,
            record_index: record_index as u32,
        });
        let mut palette = [0u64; 16];
        let mut palette_len = 0usize;
        let mut raw_timestamp_len = 0usize;
        let mut palette_available = true;
        for pair in group.windows(2) {
            let delta = pair[1].timestamp_ns - pair[0].timestamp_ns;
            raw_timestamp_len += encoded_len(delta);
            if palette[..palette_len].contains(&delta) {
                continue;
            }
            if palette_len == palette.len() {
                palette_available = false;
                continue;
            }
            palette[palette_len] = delta;
            palette_len += 1;
        }

        let raw_len = 1 + group.len() + raw_timestamp_len;
        let palette_bits = palette_index_bits(palette_len);
        let palette_len_bytes = if palette_available && palette_len > 1 {
            2 + palette[..palette_len]
                .iter()
                .map(|&delta| encoded_len(delta))
                .sum::<usize>()
                + (group.len().saturating_sub(1) * palette_bits as usize).div_ceil(8)
                + group.len()
        } else {
            usize::MAX
        };

        if palette_len == 1 {
            records.push(1);
            encode_u64(palette[0], &mut records);
            records.extend(group.iter().map(|word| word.value as u8));
        } else if palette_len_bytes < raw_len {
            records.push(2);
            records.push(palette_len as u8);
            for &delta in &palette[..palette_len] {
                encode_u64(delta, &mut records);
            }
            let packed_offset = records.len();
            let packed_len = (group.len().saturating_sub(1) * palette_bits as usize).div_ceil(8);
            records.resize(packed_offset + packed_len, 0);
            for (delta_index, pair) in group.windows(2).enumerate() {
                let delta = pair[1].timestamp_ns - pair[0].timestamp_ns;
                let palette_index = palette[..palette_len]
                    .iter()
                    .position(|&candidate| candidate == delta)
                    .expect("every delta was added to the palette");
                write_packed_index(
                    &mut records[packed_offset..packed_offset + packed_len],
                    delta_index,
                    palette_bits,
                    palette_index as u8,
                );
            }
            records.extend(group.iter().map(|word| word.value as u8));
        } else {
            records.push(0);
            for (index, word) in group.iter().enumerate() {
                if index > 0 {
                    encode_u64(
                        word.timestamp_ns - group[index - 1].timestamp_ns,
                        &mut records,
                    );
                }
                records.push(word.value as u8);
            }
        }
    }

    (records.len() < legacy_len).then_some((records, restarts))
}

fn palette_index_bits(palette_len: usize) -> u8 {
    match palette_len {
        0 | 1 => 0,
        2 => 1,
        3..=4 => 2,
        5..=8 => 3,
        _ => 4,
    }
}

fn write_packed_index(bytes: &mut [u8], index: usize, bits: u8, value: u8) {
    let bit_offset = index * bits as usize;
    let byte_index = bit_offset / 8;
    let shift = bit_offset % 8;
    let encoded = (value as u16) << shift;
    bytes[byte_index] |= encoded as u8;
    if shift + bits as usize > 8 {
        bytes[byte_index + 1] |= (encoded >> 8) as u8;
    }
}

pub(crate) fn decode_word_block(bytes: &[u8]) -> CodecResult<DecodedWordBlock> {
    let parsed = parse_word_block(bytes)?;
    let header = parsed.header;
    let mut words = Vec::with_capacity(header.word_count as usize);
    {
        let mut decoder = RecordDecoder::new(bytes, &parsed, 0, false);
        for record_index in 0..header.word_count as usize {
            words.push(decoder.next(record_index)?);
        }
        if decoder.cursor != parsed.record_end
            || decoder.next_restart_index != parsed.restarts.len()
            || !decoder.group_timestamps.is_complete()
        {
            return Err(invalid("record payload length is inconsistent"));
        }
        if decoder.timestamp != header.last_timestamp_ns {
            return Err(invalid("last timestamp does not match block header"));
        }
    }

    apply_durations(bytes, &parsed, 0, &mut words)?;
    let restarts = parsed.restarts;

    Ok(DecodedWordBlock {
        header,
        restarts,
        words,
    })
}

/// Decodes only the records needed around a time window, beginning at the
/// nearest restart entry rather than at the start of the block. The result
/// includes two predecessors and one successor when available. Two prior
/// timestamps are required to infer the cadence before a long word gap.
pub(crate) fn decode_word_block_range(
    bytes: &[u8],
    start_ns: u64,
    end_ns: u64,
    max_context_words: usize,
) -> CodecResult<DecodedWordRange> {
    if start_ns > end_ns {
        return Err(invalid("range start is after range end"));
    }
    if max_context_words == 0 {
        return Err(CodecError::InvalidConfiguration(
            "max_context_words must be greater than zero",
        ));
    }
    let parsed = parse_word_block(bytes)?;
    // Start one restart before an exact match so the predecessor that closes
    // at the query boundary is available to the renderer.
    let restart_index = parsed
        .restarts
        .partition_point(|restart| restart.timestamp_ns < start_ns)
        .saturating_sub(1);
    let restart = parsed.restarts[restart_index];
    let mut decoder = RecordDecoder::new(bytes, &parsed, restart_index, true);
    let mut previous_predecessor = None;
    let mut predecessor = None;
    let mut selected: Vec<(usize, Word)> = Vec::new();
    let mut decoded_records = 0usize;
    let mut complete = true;

    for record_index in restart.record_index as usize..parsed.header.word_count as usize {
        let word = decoder.next(record_index)?;
        decoded_records += 1;
        let timestamp = word.timestamp_ns;
        if timestamp < start_ns {
            previous_predecessor = predecessor;
            predecessor = Some((record_index, word));
            continue;
        }
        if selected.is_empty() {
            if let Some(previous) = previous_predecessor.take() {
                selected.push(previous);
            }
            if let Some(previous) = predecessor.take()
                && selected.len() < max_context_words
            {
                selected.push(previous);
            }
        }
        if selected.len() >= max_context_words {
            complete = false;
            break;
        }
        selected.push((record_index, word));
        if timestamp > end_ns {
            break;
        }
    }
    if selected.is_empty() {
        if let Some(previous) = previous_predecessor {
            selected.push(previous);
        }
        if let Some(previous) = predecessor
            && selected.len() < max_context_words
        {
            selected.push(previous);
        }
    }

    let first_record_index = selected.first().map_or(0, |(index, _)| *index);
    let mut words: Vec<_> = selected.into_iter().map(|(_, word)| word).collect();
    apply_durations(bytes, &parsed, first_record_index, &mut words)?;
    Ok(DecodedWordRange {
        header: parsed.header,
        words,
        complete,
        decoded_records,
    })
}

struct ParsedWordBlock {
    header: WordBlockHeader,
    restarts: Vec<RestartEntry>,
    record_end: usize,
    duration_offset: usize,
    block_len: usize,
    value_bytes: usize,
}

enum GroupTimestampDecoder {
    Raw,
    Constant(u64),
    Palette {
        deltas: [u64; 16],
        palette_len: usize,
        bits: u8,
        packed_offset: usize,
        delta_count: usize,
        next_delta: usize,
    },
}

impl GroupTimestampDecoder {
    fn is_complete(&self) -> bool {
        match self {
            Self::Raw | Self::Constant(_) => true,
            Self::Palette {
                delta_count,
                next_delta,
                ..
            } => next_delta == delta_count,
        }
    }
}

struct RecordDecoder<'a> {
    bytes: &'a [u8],
    parsed: &'a ParsedWordBlock,
    cursor: usize,
    timestamp: u64,
    next_restart_index: usize,
    group_timestamps: GroupTimestampDecoder,
    skip_initial_legacy_delta: bool,
}

impl<'a> RecordDecoder<'a> {
    fn new(
        bytes: &'a [u8],
        parsed: &'a ParsedWordBlock,
        restart_index: usize,
        skip_initial_legacy_delta: bool,
    ) -> Self {
        let restart = parsed.restarts[restart_index];
        Self {
            bytes,
            parsed,
            cursor: BLOCK_HEADER_SIZE + restart.payload_offset as usize,
            timestamp: restart.timestamp_ns,
            next_restart_index: restart_index,
            group_timestamps: GroupTimestampDecoder::Raw,
            skip_initial_legacy_delta,
        }
    }

    fn next(&mut self, record_index: usize) -> CodecResult<Word> {
        let payload_offset = (self.cursor - BLOCK_HEADER_SIZE) as u32;
        let at_restart = self
            .parsed
            .restarts
            .get(self.next_restart_index)
            .is_some_and(|restart| restart.record_index as usize == record_index);
        if self.parsed.header.flags & BLOCK_FLAG_GROUPED_TIMESTAMPS != 0 {
            if at_restart {
                if !self.group_timestamps.is_complete() {
                    return Err(invalid(
                        "timestamp palette extends beyond its restart group",
                    ));
                }
                let restart = self.parsed.restarts[self.next_restart_index];
                if restart.payload_offset != payload_offset {
                    return Err(invalid("restart entry does not match record payload"));
                }
                self.timestamp = restart.timestamp_ns;
                let mode = *self.bytes.get(self.cursor).ok_or(CodecError::Truncated)?;
                self.cursor += 1;
                self.group_timestamps = match mode {
                    0 => GroupTimestampDecoder::Raw,
                    1 => GroupTimestampDecoder::Constant(decode_u64(
                        &self.bytes[..self.parsed.record_end],
                        &mut self.cursor,
                    )?),
                    2 => self.decode_timestamp_palette(record_index)?,
                    _ => return Err(invalid("invalid grouped timestamp mode")),
                };
                self.next_restart_index += 1;
            } else {
                let delta = match &mut self.group_timestamps {
                    GroupTimestampDecoder::Raw => {
                        decode_u64(&self.bytes[..self.parsed.record_end], &mut self.cursor)?
                    }
                    GroupTimestampDecoder::Constant(delta) => *delta,
                    GroupTimestampDecoder::Palette {
                        deltas,
                        palette_len,
                        bits,
                        packed_offset,
                        delta_count,
                        next_delta,
                    } => {
                        if *next_delta >= *delta_count {
                            return Err(invalid("timestamp palette is shorter than its group"));
                        }
                        let palette_index =
                            read_packed_index(self.bytes, *packed_offset, *next_delta, *bits)
                                as usize;
                        *next_delta += 1;
                        *deltas
                            .get(palette_index)
                            .filter(|_| palette_index < *palette_len)
                            .ok_or_else(|| invalid("timestamp palette index is out of bounds"))?
                    }
                };
                self.timestamp = self
                    .timestamp
                    .checked_add(delta)
                    .ok_or_else(|| invalid("timestamp delta overflow"))?;
            }
        } else {
            let delta = decode_u64(&self.bytes[..self.parsed.record_end], &mut self.cursor)?;
            if self.skip_initial_legacy_delta {
                if record_index == 0 && delta != 0 {
                    return Err(invalid("first timestamp delta is not zero"));
                }
                self.skip_initial_legacy_delta = false;
            } else if record_index == 0 {
                if delta != 0 {
                    return Err(invalid("first timestamp delta is not zero"));
                }
            } else {
                self.timestamp = self
                    .timestamp
                    .checked_add(delta)
                    .ok_or_else(|| invalid("timestamp delta overflow"))?;
            }
            if at_restart {
                let restart = self.parsed.restarts[self.next_restart_index];
                if restart.timestamp_ns != self.timestamp
                    || restart.payload_offset != payload_offset
                {
                    return Err(invalid("restart entry does not match record payload"));
                }
                self.next_restart_index += 1;
            }
        }

        let value = read_value(
            self.bytes,
            &mut self.cursor,
            self.parsed.record_end,
            self.parsed.value_bytes,
        )?;
        Ok(Word::new(value, self.timestamp))
    }

    fn decode_timestamp_palette(
        &mut self,
        record_index: usize,
    ) -> CodecResult<GroupTimestampDecoder> {
        let palette_len = *self.bytes.get(self.cursor).ok_or(CodecError::Truncated)? as usize;
        self.cursor += 1;
        if !(2..=16).contains(&palette_len) {
            return Err(invalid("invalid timestamp palette length"));
        }
        let mut deltas = [0u64; 16];
        for delta in &mut deltas[..palette_len] {
            *delta = decode_u64(&self.bytes[..self.parsed.record_end], &mut self.cursor)?;
        }
        let group_end = self
            .parsed
            .restarts
            .get(self.next_restart_index + 1)
            .map_or(self.parsed.header.word_count as usize, |restart| {
                restart.record_index as usize
            });
        let delta_count = group_end
            .checked_sub(record_index + 1)
            .ok_or_else(|| invalid("timestamp restart group is invalid"))?;
        let bits = palette_index_bits(palette_len);
        let packed_len = (delta_count * bits as usize).div_ceil(8);
        let packed_offset = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(packed_len)
            .filter(|&cursor| cursor <= self.parsed.record_end)
            .ok_or(CodecError::Truncated)?;
        Ok(GroupTimestampDecoder::Palette {
            deltas,
            palette_len,
            bits,
            packed_offset,
            delta_count,
            next_delta: 0,
        })
    }
}

fn read_packed_index(bytes: &[u8], packed_offset: usize, index: usize, bits: u8) -> u8 {
    let bit_offset = index * bits as usize;
    let byte_index = packed_offset + bit_offset / 8;
    let shift = bit_offset % 8;
    let low = bytes[byte_index] as u16;
    let high = bytes.get(byte_index + 1).copied().unwrap_or(0) as u16;
    (((low | high << 8) >> shift) & ((1u16 << bits) - 1)) as u8
}

fn parse_word_block(bytes: &[u8]) -> CodecResult<ParsedWordBlock> {
    let header = WordBlockHeader::from_bytes(bytes)?;
    let block_len = header.block_len as usize;
    if block_len != bytes.len() || block_len < BLOCK_HEADER_SIZE {
        return Err(invalid("word-block length does not match its header"));
    }
    let actual_checksum = block_checksum(bytes, BLOCK_CHECKSUM_OFFSET);
    if actual_checksum != header.crc32c {
        return Err(CodecError::ChecksumMismatch {
            expected: header.crc32c,
            actual: actual_checksum,
        });
    }
    if header.word_count == 0 {
        return Err(CodecError::EmptyBlock);
    }
    let value_bytes = header.value_bytes as usize;
    if !matches!(value_bytes, 1 | 2 | 4 | 8) {
        return Err(invalid("invalid value width"));
    }
    if header.flags
        & !(BLOCK_FLAG_HAS_DURATIONS | BLOCK_FLAG_HAS_PAYLOADS | BLOCK_FLAG_GROUPED_TIMESTAMPS)
        != 0
    {
        return Err(invalid("word block contains unsupported flags"));
    }

    let record_end = BLOCK_HEADER_SIZE
        .checked_add(header.record_payload_len as usize)
        .ok_or_else(|| invalid("record payload offset overflow"))?;
    let restart_offset = header.restart_table_offset as usize;
    let restart_bytes = (header.restart_count as usize)
        .checked_mul(RESTART_ENTRY_SIZE)
        .ok_or_else(|| invalid("restart table size overflow"))?;
    let restart_end = restart_offset
        .checked_add(restart_bytes)
        .ok_or_else(|| invalid("restart table offset overflow"))?;
    let duration_offset = header.duration_table_offset as usize;
    if restart_offset != record_end || duration_offset != restart_end || duration_offset > block_len
    {
        return Err(invalid("word-block table offsets are inconsistent"));
    }

    let mut restarts = Vec::with_capacity(header.restart_count as usize);
    for index in 0..header.restart_count as usize {
        restarts.push(RestartEntry::read_from(
            bytes,
            restart_offset + index * RESTART_ENTRY_SIZE,
        )?);
    }
    validate_restart_order(&restarts, header.word_count, header.record_payload_len)?;
    Ok(ParsedWordBlock {
        header,
        restarts,
        record_end,
        duration_offset,
        block_len,
        value_bytes,
    })
}

fn apply_durations(
    bytes: &[u8],
    parsed: &ParsedWordBlock,
    first_record_index: usize,
    words: &mut [Word],
) -> CodecResult<()> {
    let mut duration_cursor = parsed.duration_offset;
    let mut previous_duration_index = 0usize;
    for exception_index in 0..parsed.header.duration_count as usize {
        let index_delta = decode_u64(bytes, &mut duration_cursor)?;
        let record_index = if exception_index == 0 {
            usize::try_from(index_delta).map_err(|_| invalid("duration index overflow"))?
        } else {
            if index_delta == 0 {
                return Err(invalid("duration exception indices are not increasing"));
            }
            previous_duration_index
                .checked_add(
                    usize::try_from(index_delta).map_err(|_| invalid("duration index overflow"))?,
                )
                .ok_or_else(|| invalid("duration index overflow"))?
        };
        let duration_ns = decode_u64(bytes, &mut duration_cursor)?;
        if duration_ns == 0 {
            return Err(invalid("zero duration stored as an exception"));
        }
        if record_index >= parsed.header.word_count as usize {
            return Err(invalid("duration exception index is out of bounds"));
        }
        if let Some(local_index) = record_index.checked_sub(first_record_index)
            && let Some(word) = words.get_mut(local_index)
        {
            word.duration_ns = duration_ns;
        }
        previous_duration_index = record_index;
    }
    let mut payload_cursor = duration_cursor;
    if parsed.header.flags & BLOCK_FLAG_HAS_PAYLOADS != 0 {
        let payload_count = usize::try_from(decode_u64(bytes, &mut payload_cursor)?)
            .map_err(|_| invalid("word payload count overflow"))?;
        let mut previous_payload_index = 0usize;
        for payload_index in 0..payload_count {
            let index_delta = usize::try_from(decode_u64(bytes, &mut payload_cursor)?)
                .map_err(|_| invalid("word payload index overflow"))?;
            let record_index = if payload_index == 0 {
                index_delta
            } else {
                if index_delta == 0 {
                    return Err(invalid("word payload indices are not increasing"));
                }
                previous_payload_index
                    .checked_add(index_delta)
                    .ok_or_else(|| invalid("word payload index overflow"))?
            };
            if record_index >= parsed.header.word_count as usize {
                return Err(invalid("word payload index is out of bounds"));
            }
            let kind = *bytes.get(payload_cursor).ok_or(CodecError::Truncated)?;
            payload_cursor += 1;
            let length = usize::try_from(decode_u64(bytes, &mut payload_cursor)?)
                .map_err(|_| invalid("word payload length overflow"))?;
            let end = payload_cursor
                .checked_add(length)
                .ok_or_else(|| invalid("word payload length overflow"))?;
            let value = bytes
                .get(payload_cursor..end)
                .ok_or(CodecError::Truncated)?;
            let payload = match kind {
                1 => WordPayload::Bytes(value.into()),
                2 => WordPayload::Text(
                    std::str::from_utf8(value)
                        .map_err(|_| invalid("word text payload is not UTF-8"))?
                        .into(),
                ),
                _ => return Err(invalid("unknown word payload kind")),
            };
            if let Some(local_index) = record_index.checked_sub(first_record_index)
                && let Some(word) = words.get_mut(local_index)
            {
                word.payload = Some(payload);
            }
            payload_cursor = end;
            previous_payload_index = record_index;
        }
    }
    let padding = bytes
        .get(payload_cursor..parsed.block_len)
        .ok_or(CodecError::Truncated)?;
    if padding.len() > 7 || padding.iter().any(|&byte| byte != 0) {
        return Err(invalid("invalid word-block padding"));
    }

    Ok(())
}

#[cfg(test)]
fn validate_order(words: &[Word]) -> CodecResult<()> {
    for (index, pair) in words.windows(2).enumerate() {
        if pair[1].timestamp_ns < pair[0].timestamp_ns {
            return Err(CodecError::OutOfOrder {
                index: index + 1,
                previous_timestamp_ns: pair[0].timestamp_ns,
                timestamp_ns: pair[1].timestamp_ns,
            });
        }
    }
    Ok(())
}

fn validate_restart_order(
    restarts: &[RestartEntry],
    word_count: u32,
    payload_len: u32,
) -> CodecResult<()> {
    if restarts.is_empty() || restarts[0].record_index != 0 || restarts[0].payload_offset != 0 {
        return Err(invalid("restart table does not begin at the first record"));
    }
    for pair in restarts.windows(2) {
        if pair[1].record_index <= pair[0].record_index
            || pair[1].payload_offset <= pair[0].payload_offset
            || pair[1].timestamp_ns < pair[0].timestamp_ns
        {
            return Err(invalid("restart entries are not strictly ordered"));
        }
    }
    if restarts.last().is_some_and(|restart| {
        restart.record_index >= word_count || restart.payload_offset >= payload_len
    }) {
        return Err(invalid("restart entry is outside the record payload"));
    }
    Ok(())
}

fn value_width(max_value: u64) -> usize {
    match max_value {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        0x1_0000..=0xffff_ffff => 4,
        _ => 8,
    }
}

fn payload_len(payload: &WordPayload) -> usize {
    match payload {
        WordPayload::Bytes(bytes) => bytes.len(),
        WordPayload::Text(text) => text.len(),
    }
}

fn append_value(value: u64, width: usize, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_le_bytes()[..width]);
}

fn read_value(
    bytes: &[u8],
    cursor: &mut usize,
    record_end: usize,
    width: usize,
) -> CodecResult<u64> {
    let end = cursor.checked_add(width).ok_or(CodecError::Truncated)?;
    let encoded = bytes
        .get(*cursor..end.min(record_end))
        .filter(|encoded| encoded.len() == width)
        .ok_or(CodecError::Truncated)?;
    let mut value = [0u8; 8];
    value[..width].copy_from_slice(encoded);
    *cursor = end;
    Ok(u64::from_le_bytes(value))
}

fn to_u32(value: usize, what: &str) -> CodecResult<u32> {
    u32::try_from(value).map_err(|_| invalid(&format!("{what} exceeds 4 GiB")))
}

fn invalid(message: &str) -> CodecError {
    CodecError::InvalidFormat(message.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const DEFAULT_RESTART_INTERVAL: usize = 512;

    fn encode_word_block(
        sequence: u64,
        words: &[Word],
        output: &mut Vec<u8>,
    ) -> CodecResult<EncodedBlockMetadata> {
        encode_word_block_with_interval(sequence, words, DEFAULT_RESTART_INTERVAL, output)
    }

    /// Finds the first restart at an equal timestamp, or the last restart
    /// before the requested timestamp.
    fn find_restart_for_timestamp(
        restarts: &[RestartEntry],
        timestamp_ns: u64,
    ) -> Option<RestartEntry> {
        let first_not_less = restarts.partition_point(|entry| entry.timestamp_ns < timestamp_ns);
        if restarts
            .get(first_not_less)
            .is_some_and(|entry| entry.timestamp_ns == timestamp_ns)
        {
            return Some(restarts[first_not_less]);
        }
        first_not_less.checked_sub(1).map(|index| restarts[index])
    }

    fn round_trip(words: &[Word]) -> (EncodedBlockMetadata, Vec<u8>) {
        let mut bytes = Vec::new();
        let metadata = encode_word_block(17, words, &mut bytes).unwrap();
        let decoded = decode_word_block(&bytes).unwrap();
        assert_eq!(decoded.header, metadata.header);
        assert_eq!(decoded.restarts, metadata.restarts);
        assert_eq!(decoded.words, words);
        (metadata, bytes)
    }

    #[test]
    fn block_round_trips_widths_equal_timestamps_and_durations() {
        let words = [
            Word::new(0xff, 100),
            Word::spanning(0x100, 100, 25),
            Word::new(0xffff_ffff, 180),
            Word::spanning(u64::MAX, 1_000_000, u64::MAX),
        ];
        let (metadata, _) = round_trip(&words);
        assert_eq!(metadata.header.value_bytes, 8);
        assert_eq!(metadata.header.duration_count, 2);
    }

    #[test]
    fn block_and_range_round_trip_arbitrary_width_and_text_words() {
        let bytes = Arc::<[u8]>::from((0..=255).collect::<Vec<_>>());
        let words = [
            Word::new(7, 100),
            Word::bytes_with_tag(3, Arc::clone(&bytes), 200, 25),
            Word::labeled(9, "decoder label", 300, 40),
            Word::new(11, 400),
        ];
        let (_, encoded) = round_trip(&words);

        let range = decode_word_block_range(&encoded, 200, 300, 8).unwrap();
        assert_eq!(range.words, words);
        assert_eq!(
            range.words[1].payload,
            Some(WordPayload::Bytes(Arc::clone(&bytes)))
        );
    }

    #[test]
    fn value_width_uses_the_narrowest_supported_representation() {
        for (value, expected_width) in [
            (0xff, 1),
            (0x100, 2),
            (0xffff, 2),
            (0x1_0000, 4),
            (0xffff_ffff, 4),
            (0x1_0000_0000, 8),
        ] {
            let (metadata, _) = round_trip(&[Word::new(value, 0)]);
            assert_eq!(metadata.header.value_bytes, expected_width);
        }
    }

    #[test]
    fn randomized_ordered_words_round_trip() {
        let mut random = 0x6a09_e667_f3bc_c909u64;
        for case in 0..64 {
            let count = 1 + (next_random(&mut random) as usize % 2_000);
            let mut timestamp = next_random(&mut random) % 10_000;
            let mut words = Vec::with_capacity(count);
            for index in 0..count {
                timestamp = timestamp.saturating_add(next_random(&mut random) % 1_000);
                let value = match case % 4 {
                    0 => next_random(&mut random) & 0xff,
                    1 => next_random(&mut random) & 0xffff,
                    2 => next_random(&mut random) & 0xffff_ffff,
                    _ => next_random(&mut random),
                };
                let duration = if index % 17 == 0 {
                    next_random(&mut random) % 10_000 + 1
                } else {
                    0
                };
                words.push(Word::spanning(value, timestamp, duration));
            }
            round_trip(&words);
        }
    }

    #[test]
    fn constant_cadence_eight_bit_payload_is_near_one_byte_per_word() {
        let words: Vec<_> = (0..DEFAULT_MAX_WORDS_PER_BLOCK)
            .map(|index| Word::new((index & 0xff) as u64, index as u64 * 80))
            .collect();
        let (metadata, _) = round_trip(&words);
        assert_ne!(metadata.header.flags & BLOCK_FLAG_GROUPED_TIMESTAMPS, 0);
        let bytes_per_word = metadata.header.record_payload_len as f64 / words.len() as f64;
        assert!(bytes_per_word <= 1.01, "{bytes_per_word} bytes/word");
    }

    #[test]
    fn small_delta_palette_is_bit_packed() {
        let deltas = [60, 100, 80, 120];
        let mut timestamp = 0u64;
        let words: Vec<_> = (0..DEFAULT_MAX_WORDS_PER_BLOCK)
            .map(|index| {
                if index > 0 {
                    timestamp += deltas[index % deltas.len()];
                }
                Word::new((index & 0xff) as u64, timestamp)
            })
            .collect();
        let (metadata, _) = round_trip(&words);
        assert_ne!(metadata.header.flags & BLOCK_FLAG_GROUPED_TIMESTAMPS, 0);
        let bytes_per_word = metadata.header.record_payload_len as f64 / words.len() as f64;
        assert!(bytes_per_word <= 1.27, "{bytes_per_word} bytes/word");
    }

    #[test]
    fn grouped_timestamp_blocks_preserve_irregular_groups_and_range_queries() {
        let mut timestamp = 100u64;
        let words: Vec<_> = (0..2_000)
            .map(|index| {
                if index > 0 {
                    timestamp += if index == 700 { 11 } else { 10 };
                }
                Word::new((index & 0xff) as u64, timestamp)
            })
            .collect();
        let mut bytes = Vec::new();
        let metadata = encode_word_block(0, &words, &mut bytes).unwrap();
        assert_ne!(metadata.header.flags & BLOCK_FLAG_GROUPED_TIMESTAMPS, 0);
        assert_eq!(decode_word_block(&bytes).unwrap().words, words);

        let range =
            decode_word_block_range(&bytes, words[690].timestamp_ns, words[710].timestamp_ns, 64)
                .unwrap();
        assert_eq!(range.words, words[688..=711]);
    }

    #[test]
    fn decoder_rejects_unknown_grouped_timestamp_modes() {
        let words: Vec<_> = (0..1_000)
            .map(|index| Word::new((index & 0xff) as u64, index as u64 * 10))
            .collect();
        let (_, mut bytes) = round_trip(&words);
        bytes[BLOCK_HEADER_SIZE] = 3;
        bytes[BLOCK_CHECKSUM_OFFSET..BLOCK_CHECKSUM_OFFSET + 4].fill(0);
        let checksum = block_checksum(&bytes, BLOCK_CHECKSUM_OFFSET);
        bytes[BLOCK_CHECKSUM_OFFSET..BLOCK_CHECKSUM_OFFSET + 4]
            .copy_from_slice(&checksum.to_le_bytes());

        assert!(matches!(
            decode_word_block(&bytes),
            Err(CodecError::InvalidFormat(_))
        ));
    }

    #[test]
    fn restart_entries_bound_forward_decode_distance() {
        let words: Vec<_> = (0..1_000)
            .map(|index| Word::new(index as u64, index as u64 * 10))
            .collect();
        let (metadata, _) = round_trip(&words);
        assert_eq!(
            metadata.restarts.len(),
            words.len().div_ceil(DEFAULT_RESTART_INTERVAL)
        );
        assert_eq!(
            metadata.restarts[1].record_index as usize,
            DEFAULT_RESTART_INTERVAL
        );
    }

    #[test]
    fn range_decode_starts_at_restart_and_keeps_boundary_context() {
        let words: Vec<_> = (0..2_000)
            .map(|index| {
                if index == 1_505 {
                    Word::spanning(index as u64, index as u64 * 10, 7)
                } else {
                    Word::new(index as u64, index as u64 * 10)
                }
            })
            .collect();
        let mut bytes = Vec::new();
        encode_word_block(0, &words, &mut bytes).unwrap();

        let range = decode_word_block_range(&bytes, 15_000, 15_100, 32).unwrap();
        assert!(range.complete);
        assert_eq!(range.words, words[1_498..=1_511]);
        assert!(
            range.decoded_records <= DEFAULT_RESTART_INTERVAL + 12,
            "decoded {} records",
            range.decoded_records
        );

        let boundary = decode_word_block_range(&bytes, 15_360, 15_370, 8).unwrap();
        assert_eq!(boundary.words, words[1_534..=1_538]);
        assert!(boundary.decoded_records <= DEFAULT_RESTART_INTERVAL + 3);
    }

    #[test]
    fn restart_search_preserves_words_at_duplicate_query_timestamps() {
        let restarts = [
            RestartEntry {
                timestamp_ns: 10,
                payload_offset: 0,
                record_index: 0,
            },
            RestartEntry {
                timestamp_ns: 20,
                payload_offset: 100,
                record_index: 256,
            },
            RestartEntry {
                timestamp_ns: 20,
                payload_offset: 200,
                record_index: 512,
            },
            RestartEntry {
                timestamp_ns: 30,
                payload_offset: 300,
                record_index: 768,
            },
        ];

        assert_eq!(find_restart_for_timestamp(&restarts, 9), None);
        assert_eq!(find_restart_for_timestamp(&restarts, 10), Some(restarts[0]));
        assert_eq!(find_restart_for_timestamp(&restarts, 20), Some(restarts[1]));
        assert_eq!(find_restart_for_timestamp(&restarts, 25), Some(restarts[2]));
        assert_eq!(find_restart_for_timestamp(&restarts, 30), Some(restarts[3]));
    }

    #[test]
    fn builder_reports_word_gap_and_payload_boundaries_without_consuming_word() {
        let mut builder = WordBlockBuilder::new(BlockCodecConfig {
            max_words: 2,
            max_inter_word_gap_ns: 100,
            ..BlockCodecConfig::default()
        })
        .unwrap();
        assert_eq!(builder.push(Word::new(1, 0)).unwrap(), PushResult::Appended);
        assert_eq!(
            builder.push(Word::new(2, 10)).unwrap(),
            PushResult::Appended
        );
        assert_eq!(
            builder.push(Word::new(3, 20)).unwrap(),
            PushResult::BlockFull
        );
        assert_eq!(builder.len(), 2);

        builder.clear();
        assert_eq!(builder.push(Word::new(1, 0)).unwrap(), PushResult::Appended);
        assert_eq!(
            builder.push(Word::new(2, 101)).unwrap(),
            PushResult::BlockFull
        );
        assert_eq!(builder.words(), &[Word::new(1, 0)]);
    }

    #[test]
    fn builder_batch_extension_stops_at_the_same_boundaries() {
        let config = BlockCodecConfig {
            max_words: 3,
            max_inter_word_gap_ns: 100,
            ..BlockCodecConfig::default()
        };
        let words = [
            Word::new(1, 0),
            Word::spanning(2, 10, 3),
            Word::new(3, 20),
            Word::new(4, 200),
        ];
        let mut batch = WordBlockBuilder::new(config).unwrap();
        assert_eq!(batch.extend_ordered(&words), 3);
        assert_eq!(batch.words(), &words[..3]);
        assert_eq!(batch.extend_ordered(&words[3..]), 0);

        let mut scalar = WordBlockBuilder::new(config).unwrap();
        for word in &words[..3] {
            assert_eq!(scalar.push(word.clone()).unwrap(), PushResult::Appended);
        }
        assert_eq!(
            scalar.push(words[3].clone()).unwrap(),
            PushResult::BlockFull
        );

        let mut batch_bytes = Vec::new();
        let mut scalar_bytes = Vec::new();
        batch.encode(0, &mut batch_bytes).unwrap();
        scalar.encode(0, &mut scalar_bytes).unwrap();
        assert_eq!(batch_bytes, scalar_bytes);
    }

    #[test]
    fn encoding_rejects_empty_and_out_of_order_blocks() {
        let mut bytes = Vec::new();
        assert_eq!(
            encode_word_block(0, &[], &mut bytes),
            Err(CodecError::EmptyBlock)
        );
        assert!(matches!(
            encode_word_block(0, &[Word::new(0, 2), Word::new(0, 1)], &mut bytes),
            Err(CodecError::OutOfOrder { index: 1, .. })
        ));
    }

    #[test]
    fn decoder_rejects_corruption_and_truncation() {
        let words = [Word::new(7, 10), Word::spanning(8, 20, 3)];
        let (_, mut bytes) = round_trip(&words);
        bytes[BLOCK_HEADER_SIZE + 1] ^= 0x80;
        assert!(matches!(
            decode_word_block(&bytes),
            Err(CodecError::ChecksumMismatch { .. })
        ));

        bytes.pop();
        assert!(decode_word_block(&bytes).is_err());
    }

    #[test]
    fn decoder_validates_restart_structure_after_checksum() {
        let words: Vec<_> = (0..DEFAULT_RESTART_INTERVAL + 44)
            .map(|index| Word::new(index as u64, index as u64 * 10))
            .collect();
        let (metadata, mut bytes) = round_trip(&words);
        let second_restart_index_offset =
            metadata.header.restart_table_offset as usize + RESTART_ENTRY_SIZE + 12;
        bytes[second_restart_index_offset..second_restart_index_offset + 4]
            .copy_from_slice(&0u32.to_le_bytes());
        bytes[BLOCK_CHECKSUM_OFFSET..BLOCK_CHECKSUM_OFFSET + 4].fill(0);
        let checksum = block_checksum(&bytes, BLOCK_CHECKSUM_OFFSET);
        bytes[BLOCK_CHECKSUM_OFFSET..BLOCK_CHECKSUM_OFFSET + 4]
            .copy_from_slice(&checksum.to_le_bytes());

        assert!(matches!(
            decode_word_block(&bytes),
            Err(CodecError::InvalidFormat(_))
        ));
    }

    fn next_random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }
}
