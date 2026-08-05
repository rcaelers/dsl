//! Extracts one contiguous bit field from each input word.

use std::collections::VecDeque;

use signal_processing::{Word, WordPayload};
use signal_runtime::{
    InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, WorkError, WorkResult,
};

/// Extracts `bit_count` bits beginning at `first_bit`, where bit zero is the
/// least-significant bit. This is equivalent to a right shift followed by a
/// width mask.
///
/// Numeric words use [`Word::value`]. Byte-backed words are interpreted as a
/// big-endian bit string, matching their display order. Fields up to 64 bits
/// are emitted as numeric words; wider fields retain a byte payload and carry
/// their low 64 bits in `Word::value`.
pub struct WordFieldExtractor {
    name: String,
    first_bit: usize,
    bit_count: usize,
    input_buffer: VecDeque<Word>,
}

impl WordFieldExtractor {
    /// Creates a word-field extractor with the supplied bit-range configuration.
    ///
    /// # Parameters
    /// - `first_bit`: Input consumed by this operation.
    /// - `bit_count`: Input consumed by this operation.
    pub fn new(first_bit: usize, bit_count: usize) -> Self {
        Self {
            name: "word_field_extractor".to_owned(),
            first_bit,
            bit_count: bit_count.max(1),
            input_buffer: VecDeque::new(),
        }
    }

    /// Returns this value configured with name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn source_bit(word: &Word, bit: usize) -> bool {
        match &word.payload {
            Some(WordPayload::Bytes(bytes)) => {
                let byte_from_end = bit / 8;
                if byte_from_end >= bytes.len() {
                    return false;
                }
                let byte = bytes[bytes.len() - 1 - byte_from_end];
                byte & (1 << (bit % 8)) != 0
            }
            Some(WordPayload::Text(_)) | None => {
                bit < u64::BITS as usize && word.value & (1_u64 << bit) != 0
            }
        }
    }

    fn low_value(&self, word: &Word) -> u64 {
        let mut value = 0_u64;
        for output_bit in 0..self.bit_count.min(u64::BITS as usize) {
            if Self::source_bit(word, self.first_bit.saturating_add(output_bit)) {
                value |= 1_u64 << output_bit;
            }
        }
        value
    }

    fn extracted_bytes(&self, word: &Word) -> Vec<u8> {
        let mut bytes = vec![0_u8; self.bit_count.div_ceil(8)];
        for output_bit in 0..self.bit_count {
            if Self::source_bit(word, self.first_bit.saturating_add(output_bit)) {
                let byte_from_end = output_bit / 8;
                let index = bytes.len() - 1 - byte_from_end;
                bytes[index] |= 1 << (output_bit % 8);
            }
        }
        bytes
    }

    fn extract(&self, word: Word) -> Word {
        let value = self.low_value(&word);
        if self.bit_count <= u64::BITS as usize {
            Word::spanning(value, word.timestamp_ns, word.duration_ns)
        } else {
            Word::bytes_with_tag(
                value,
                self.extracted_bytes(&word),
                word.timestamp_ns,
                word.duration_ns,
            )
        }
    }
}

impl ProcessNode for WordFieldExtractor {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        1
    }

    fn num_outputs(&self) -> usize {
        1
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        vec![PortSchema::new::<Word>("words", 0, PortDirection::Input)]
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<Word>("field", 0, PortDirection::Output)
                .with_default_buffer_capacity(8),
        ]
    }

    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        let mut input = inputs
            .first()
            .and_then(|port| port.get::<Word>(&mut self.input_buffer))
            .ok_or_else(|| WorkError::NodeError("Missing words input".to_owned()))?;
        let output = outputs
            .first()
            .and_then(|port| port.get::<Word>())
            .ok_or_else(|| WorkError::NodeError("Missing field output".to_owned()))?;

        let word = input.recv()?;
        drop(input);
        output.send(self.extract(word))?;
        Ok(1)
    }
}

#[cfg(test)]
mod implementation_tests {
    use crossbeam_channel::bounded;
    use signal_runtime::{ChannelMessage, Sender, Watchdog};

    use super::*;

    fn extract(extractor: &mut WordFieldExtractor, word: Word) -> Word {
        let watchdog = Watchdog::new();
        let (input_tx, input_rx) = bounded(1);
        input_tx.send(ChannelMessage::Sample(word)).unwrap();
        let input = InputPort::new_with_watchdog(input_rx, &watchdog, "extractor", "words");
        let (output_tx, output_rx) = bounded(1);
        let output = OutputPort::new_with_watchdog(
            Sender::new(vec![output_tx]),
            &watchdog,
            "extractor",
            "field",
        );

        assert_eq!(extractor.work(&[input], &[output]).unwrap(), 1);
        match output_rx.recv().unwrap() {
            ChannelMessage::Sample(word) => word,
            ChannelMessage::Batch(_) => panic!("extractor unexpectedly emitted a batch"),
            ChannelMessage::EndOfStream => panic!("extractor shut down without an output"),
        }
    }

    #[test]
    fn extracts_a_numeric_range_and_preserves_its_span() {
        let output = extract(
            &mut WordFieldExtractor::new(4, 4),
            Word::spanning(0xAB, 100, 25),
        );

        assert_eq!(output, Word::spanning(0xA, 100, 25));
    }

    #[test]
    fn extracts_a_range_crossing_byte_boundaries() {
        let output = extract(
            &mut WordFieldExtractor::new(4, 12),
            Word::bytes([0x12, 0x34, 0x56], 200, 30),
        );

        assert_eq!(output, Word::spanning(0x345, 200, 30));
    }

    #[test]
    fn wide_fields_remain_byte_backed_and_retain_their_low_value() {
        let output = extract(
            &mut WordFieldExtractor::new(0, 72),
            Word::bytes(
                [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x55],
                300,
                40,
            ),
        );

        assert_eq!(output.value, 0x2345_6789_ABCD_EF55);
        assert_eq!(output.timestamp_ns, 300);
        assert_eq!(output.duration_ns, 40);
        assert_eq!(
            output.payload,
            Some(WordPayload::Bytes(
                [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x55].into()
            ))
        );
    }
}
