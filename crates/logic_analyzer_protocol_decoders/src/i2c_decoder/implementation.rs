use std::collections::VecDeque;

use signal_capture::SampleBlock;
use signal_derived::{ProtocolPacket, ProtocolValue, Word};
use signal_runtime::{
    InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, WorkError, WorkResult,
};

pub const I2C_PROTOCOL_ID: &str = "i2c";

#[derive(Clone, Copy, Debug)]
struct SampledBit {
    value: bool,
    position: u64,
}

/// Decodes raw SCL/SDA samples into native words and the shared I²C packet contract.
pub struct I2cDecoder {
    name: String,
    input_buffers: [VecDeque<SampleBlock>; 2],
    previous: Option<(bool, bool)>,
    active: bool,
    repeated_start: bool,
    address_pending: bool,
    is_write: Option<bool>,
    bits: Vec<SampledBit>,
    awaiting_ack: bool,
    finished: bool,
}

impl Default for I2cDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl I2cDecoder {
    /// Creates an I²C decoder with its default configuration.
    pub fn new() -> Self {
        Self {
            name: "i2c_decoder".into(),
            input_buffers: std::array::from_fn(|_| VecDeque::new()),
            previous: None,
            active: false,
            repeated_start: false,
            address_pending: true,
            is_write: None,
            bits: Vec::with_capacity(8),
            awaiting_ack: false,
            finished: false,
        }
    }

    /// Returns this value configured with name.
    ///
    /// # Parameters
    /// - `name`: Input consumed by this operation.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn acquire_blocks(&mut self, inputs: &[InputPort]) -> WorkResult<Option<[SampleBlock; 2]>> {
        if inputs.len() != 2 {
            return Err(WorkError::NodeError(
                "I²C decoder requires SCL and SDA inputs".into(),
            ));
        }
        let mut blocks = Vec::with_capacity(2);
        for (index, (input, buffer)) in inputs.iter().zip(&mut self.input_buffers).enumerate() {
            let Some(mut receiver) = input.get::<SampleBlock>(buffer) else {
                return Err(WorkError::NodeError(format!(
                    "I²C decoder input {index} is not a sample block stream"
                )));
            };
            match receiver.recv() {
                Ok(block) => blocks.push(block),
                Err(WorkError::Shutdown) if index == 0 => return Ok(None),
                Err(WorkError::Shutdown) => {
                    return Err(WorkError::NodeError(
                        "I²C SDA input ended before SCL".into(),
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        let blocks: [SampleBlock; 2] = blocks.try_into().expect("two I²C inputs were read");
        let [scl, sda] = &blocks;
        if scl.start_position != sda.start_position
            || scl.num_samples != sda.num_samples
            || scl.timestamp_step != sda.timestamp_step
        {
            return Err(WorkError::NodeError(
                "I²C SCL and SDA sample blocks are not aligned".into(),
            ));
        }
        if scl.num_samples == 0 {
            return Err(WorkError::NodeError(
                "I²C decoder received an empty sample block".into(),
            ));
        }
        Ok(Some(blocks))
    }

    fn packet(
        &self,
        command: &str,
        data: ProtocolValue,
        start_sample: u64,
        end_sample: u64,
        timestamp_step: u64,
    ) -> ProtocolPacket {
        ProtocolPacket {
            start_sample,
            end_sample,
            start_time_ns: start_sample.saturating_mul(timestamp_step),
            end_time_ns: end_sample.saturating_mul(timestamp_step),
            protocol_id: I2C_PROTOCOL_ID.into(),
            value: ProtocolValue::List(vec![ProtocolValue::String(command.into()), data]),
        }
    }

    fn emit_control(
        &self,
        command: &str,
        position: u64,
        timestamp_step: u64,
        packets: &mut Vec<ProtocolPacket>,
    ) {
        packets.push(self.packet(
            command,
            ProtocolValue::Null,
            position,
            position,
            timestamp_step,
        ));
    }

    fn emit_byte(
        &mut self,
        timestamp_step: u64,
        words: &mut Vec<Word>,
        packets: &mut Vec<ProtocolPacket>,
    ) {
        let value = self
            .bits
            .iter()
            .fold(0u8, |value, bit| (value << 1) | u8::from(bit.value));
        let start = self.bits.first().expect("eight bits collected").position;
        let end = self.bits.last().expect("eight bits collected").position;
        let bit_span = self
            .bits
            .windows(2)
            .last()
            .map_or(1, |pair| pair[1].position.saturating_sub(pair[0].position));
        let packet_end = end.saturating_add(bit_span);
        let lsb_bits = self
            .bits
            .iter()
            .rev()
            .map(|bit| {
                ProtocolValue::List(vec![
                    ProtocolValue::Integer(i128::from(bit.value)),
                    ProtocolValue::Integer(i128::from(bit.position)),
                    ProtocolValue::Integer(i128::from(bit.position.saturating_add(bit_span))),
                ])
            })
            .collect();
        packets.push(self.packet(
            "BITS",
            ProtocolValue::List(lsb_bits),
            start,
            packet_end,
            timestamp_step,
        ));

        let (command, decoded) = if self.address_pending {
            let read = value & 1 != 0;
            self.is_write = Some(!read);
            self.address_pending = false;
            (
                if read {
                    "ADDRESS READ"
                } else {
                    "ADDRESS WRITE"
                },
                value >> 1,
            )
        } else {
            (
                if self.is_write.unwrap_or(true) {
                    "DATA WRITE"
                } else {
                    "DATA READ"
                },
                value,
            )
        };
        packets.push(self.packet(
            command,
            ProtocolValue::Integer(i128::from(decoded)),
            start,
            packet_end,
            timestamp_step,
        ));
        words.push(Word::spanning(
            u64::from(decoded),
            start.saturating_mul(timestamp_step),
            packet_end
                .saturating_sub(start)
                .saturating_mul(timestamp_step),
        ));
        self.awaiting_ack = true;
    }

    fn process_blocks(&mut self, blocks: &[SampleBlock; 2]) -> (Vec<Word>, Vec<ProtocolPacket>) {
        let [scl, sda] = blocks;
        let mut words = Vec::new();
        let mut packets = Vec::new();
        for position in scl.start_position..scl.end_position() {
            let levels = (scl.get_bit(position), sda.get_bit(position));
            let Some((previous_scl, previous_sda)) = self.previous else {
                self.previous = Some(levels);
                continue;
            };
            let (scl_high, sda_high) = levels;
            if previous_sda && !sda_high && scl_high {
                let command = if self.active || self.repeated_start {
                    "START REPEAT"
                } else {
                    "START"
                };
                self.emit_control(command, position, scl.timestamp_step, &mut packets);
                self.active = true;
                self.repeated_start = true;
                self.address_pending = true;
                self.is_write = None;
                self.bits.clear();
                self.awaiting_ack = false;
            } else if !previous_sda && sda_high && scl_high && self.active {
                self.emit_control("STOP", position, scl.timestamp_step, &mut packets);
                self.active = false;
                self.repeated_start = false;
                self.bits.clear();
                self.awaiting_ack = false;
            } else if self.active && !previous_scl && scl_high {
                if self.awaiting_ack {
                    self.emit_control(
                        if sda_high { "NACK" } else { "ACK" },
                        position,
                        scl.timestamp_step,
                        &mut packets,
                    );
                    self.awaiting_ack = false;
                    self.bits.clear();
                } else {
                    self.bits.push(SampledBit {
                        value: sda_high,
                        position,
                    });
                    if self.bits.len() == 8 {
                        self.emit_byte(scl.timestamp_step, &mut words, &mut packets);
                    }
                }
            }
            self.previous = Some(levels);
        }
        (words, packets)
    }
}

impl ProcessNode for I2cDecoder {
    fn name(&self) -> &str {
        &self.name
    }

    fn should_stop(&self) -> bool {
        self.finished
    }

    fn num_inputs(&self) -> usize {
        2
    }

    fn num_outputs(&self) -> usize {
        2
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<SampleBlock>("scl", 0, PortDirection::Input),
            PortSchema::new::<SampleBlock>("sda", 1, PortDirection::Input),
        ]
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<Word>("words", 0, PortDirection::Output)
                .with_default_buffer_capacity(8),
            PortSchema::new::<ProtocolPacket>("packets", 1, PortDirection::Output),
        ]
    }

    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        let Some(blocks) = self.acquire_blocks(inputs)? else {
            self.finished = true;
            return Err(WorkError::Shutdown);
        };
        let sample_count = blocks[0].num_samples;
        let (words, packets) = self.process_blocks(&blocks);
        if let Some(sender) = outputs.first().and_then(|output| output.get::<Word>()) {
            sender.send_batch(words)?;
        }
        if let Some(sender) = outputs
            .get(1)
            .and_then(|output| output.get::<ProtocolPacket>())
        {
            sender.send_batch(packets)?;
        }
        Ok(sample_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(scl: &[bool], sda: &[bool]) -> [SampleBlock; 2] {
        fn packed(values: &[bool]) -> Vec<u8> {
            let mut result = vec![0; values.len().div_ceil(8)];
            for (index, value) in values.iter().copied().enumerate() {
                if value {
                    result[index / 8] |= 1 << (index % 8);
                }
            }
            result
        }
        [
            SampleBlock::new(packed(scl), 0, scl.len(), 100),
            SampleBlock::new(packed(sda), 0, sda.len(), 100),
        ]
    }

    #[test]
    fn native_i2c_packets_use_the_sigrok_i2c_contract() {
        let mut scl = vec![true];
        let mut sda = vec![true];
        let mut push = |clock, data| {
            scl.push(clock);
            sda.push(data);
        };
        push(true, false); // START
        for bit in [true, false, true, false, false, false, false, false] {
            // address 0x50, write
            push(false, bit);
            push(true, bit);
        }
        push(false, false);
        push(true, false); // ACK
        push(true, true); // STOP

        let mut decoder = I2cDecoder::new();
        let (words, packets) = decoder.process_blocks(&blocks(&scl, &sda));
        assert_eq!(
            words.iter().map(|word| word.value).collect::<Vec<_>>(),
            [0x50]
        );
        let commands = packets
            .iter()
            .filter_map(|packet| match &packet.value {
                ProtocolValue::List(values) => match values.first() {
                    Some(ProtocolValue::String(command)) => Some(command.as_str()),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(commands, ["START", "BITS", "ADDRESS WRITE", "ACK", "STOP"]);
        assert!(
            packets
                .iter()
                .all(|packet| packet.protocol_id == I2C_PROTOCOL_ID)
        );
    }

    #[test]
    fn repeated_start_reframes_address_direction_data_and_acknowledgements() {
        fn push(scl: &mut Vec<bool>, sda: &mut Vec<bool>, clock: bool, data: bool) {
            scl.push(clock);
            sda.push(data);
        }
        fn byte(scl: &mut Vec<bool>, sda: &mut Vec<bool>, value: u8) {
            for bit in (0..8).rev() {
                let value = value & (1 << bit) != 0;
                push(scl, sda, false, value);
                push(scl, sda, true, value);
            }
        }
        fn acknowledge(scl: &mut Vec<bool>, sda: &mut Vec<bool>, ack: bool) {
            push(scl, sda, false, !ack);
            push(scl, sda, true, !ack);
        }

        let mut scl = vec![true];
        let mut sda = vec![true];

        push(&mut scl, &mut sda, true, false); // START
        byte(&mut scl, &mut sda, 0xa0); // 0x50 write
        acknowledge(&mut scl, &mut sda, true);
        byte(&mut scl, &mut sda, 0x12);
        acknowledge(&mut scl, &mut sda, true);
        push(&mut scl, &mut sda, false, true);
        push(&mut scl, &mut sda, true, true);
        push(&mut scl, &mut sda, true, false); // repeated START
        byte(&mut scl, &mut sda, 0xa1); // 0x50 read
        acknowledge(&mut scl, &mut sda, true);
        byte(&mut scl, &mut sda, 0xab);
        acknowledge(&mut scl, &mut sda, false);
        push(&mut scl, &mut sda, false, false);
        push(&mut scl, &mut sda, true, false);
        push(&mut scl, &mut sda, true, true); // STOP

        let mut decoder = I2cDecoder::new();
        let (words, packets) = decoder.process_blocks(&blocks(&scl, &sda));
        assert_eq!(
            words.iter().map(|word| word.value).collect::<Vec<_>>(),
            [0x50, 0x12, 0x50, 0xab]
        );
        let commands = packets
            .iter()
            .filter_map(|packet| match &packet.value {
                ProtocolValue::List(values) => match values.first() {
                    Some(ProtocolValue::String(command)) if command != "BITS" => {
                        Some(command.as_str())
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            [
                "START",
                "ADDRESS WRITE",
                "ACK",
                "DATA WRITE",
                "ACK",
                "START REPEAT",
                "ADDRESS READ",
                "ACK",
                "DATA READ",
                "NACK",
                "STOP",
            ]
        );
    }
}
