//! Groups decoded words into structured protocol-neutral packets.

use std::collections::{BTreeMap, VecDeque};

use signal_processing::{
    InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, ProtocolPacket, ProtocolValue,
    Sample, Trigger, Word, WordPayload, WorkError, WorkOutcome, WorkResult,
};

pub const PACKET_FRAME_PROTOCOL_ID: &str = "org.logicconduit.packet-frame/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatePolarity {
    ActiveHigh,
    ActiveLow,
}

/// Frames a word stream using any combination of size, delimiter, gap,
/// trigger-boundary, and level-gate conditions.
///
/// Inputs are `words`, optional `boundary` triggers, and an optional `gate`
/// level. A boundary closes the current packet before a word at the same
/// timestamp. Gate changes are also applied before same-timestamp words;
/// words outside the active gate are discarded. Remaining words are flushed
/// when all connected inputs finish.
pub struct PacketFramer {
    name: String,
    fixed_word_count: Option<usize>,
    delimiter: Option<(u64, bool)>,
    maximum_gap_ns: Option<u64>,
    maximum_words: usize,
    boundary_enabled: bool,
    gate_polarity: Option<GatePolarity>,
    gate_level: bool,
    words: Vec<Word>,
    word_buffer: VecDeque<Word>,
    boundary_buffer: VecDeque<Trigger>,
    gate_buffer: VecDeque<Sample>,
    word_head: Option<Word>,
    boundary_head: Option<Trigger>,
    gate_head: Option<Sample>,
    word_eos: bool,
    boundary_eos: bool,
    gate_eos: bool,
    finished: bool,
}

impl Default for PacketFramer {
    fn default() -> Self {
        Self::new()
    }
}

impl PacketFramer {
    pub fn new() -> Self {
        Self {
            name: "packet_framer".to_owned(),
            fixed_word_count: None,
            delimiter: None,
            maximum_gap_ns: None,
            maximum_words: 4_096,
            boundary_enabled: false,
            gate_polarity: None,
            gate_level: false,
            words: Vec::new(),
            word_buffer: VecDeque::new(),
            boundary_buffer: VecDeque::new(),
            gate_buffer: VecDeque::new(),
            word_head: None,
            boundary_head: None,
            gate_head: None,
            word_eos: false,
            boundary_eos: false,
            gate_eos: false,
            finished: false,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_fixed_word_count(mut self, count: Option<usize>) -> Self {
        self.fixed_word_count = count.filter(|count| *count > 0);
        self
    }

    pub fn with_delimiter(mut self, value: Option<u64>, include: bool) -> Self {
        self.delimiter = value.map(|value| (value, include));
        self
    }

    pub fn with_maximum_gap_ns(mut self, maximum_gap_ns: Option<u64>) -> Self {
        self.maximum_gap_ns = maximum_gap_ns.filter(|gap| *gap > 0);
        self
    }

    pub fn with_maximum_words(mut self, maximum_words: usize) -> Self {
        self.maximum_words = maximum_words.max(1);
        self
    }

    pub fn with_boundary_input(mut self, enabled: bool) -> Self {
        self.boundary_enabled = enabled;
        self
    }

    pub fn with_gate_input(mut self, polarity: Option<GatePolarity>) -> Self {
        self.gate_polarity = polarity;
        self
    }

    fn gate_active(&self) -> bool {
        match self.gate_polarity {
            None => true,
            Some(GatePolarity::ActiveHigh) => self.gate_level,
            Some(GatePolarity::ActiveLow) => !self.gate_level,
        }
    }

    fn fill_heads(&mut self, inputs: &[InputPort]) -> WorkResult<()> {
        if self.word_head.is_none() && !self.word_eos {
            let mut receiver = inputs
                .first()
                .and_then(|input| input.get::<Word>(&mut self.word_buffer))
                .ok_or_else(|| WorkError::NodeError("Missing words input".to_owned()))?;
            match receiver.recv() {
                Ok(word) => self.word_head = Some(word),
                Err(WorkError::Shutdown) => self.word_eos = true,
                Err(error) => return Err(error),
            }
        }

        if self.boundary_enabled && self.boundary_head.is_none() && !self.boundary_eos {
            let mut receiver = inputs
                .get(1)
                .and_then(|input| input.get::<Trigger>(&mut self.boundary_buffer))
                .ok_or_else(|| WorkError::NodeError("Missing boundary input".to_owned()))?;
            match receiver.recv() {
                Ok(boundary) => self.boundary_head = Some(boundary),
                Err(WorkError::Shutdown) => self.boundary_eos = true,
                Err(error) => return Err(error),
            }
        }

        if self.gate_polarity.is_some() && self.gate_head.is_none() && !self.gate_eos {
            let mut receiver = inputs
                .get(2)
                .and_then(|input| input.get::<Sample>(&mut self.gate_buffer))
                .ok_or_else(|| WorkError::NodeError("Missing gate input".to_owned()))?;
            match receiver.recv() {
                Ok(gate) => self.gate_head = Some(gate),
                Err(WorkError::Shutdown) => self.gate_eos = true,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn close_packet(&mut self) -> Option<ProtocolPacket> {
        let words = std::mem::take(&mut self.words);
        let first = words.first()?;
        let last = words.last().expect("a first word implies a last word");
        let start_time_ns = first.timestamp_ns;
        let end_time_ns = last.end_ns();
        let values = words.into_iter().map(word_value).collect();
        Some(ProtocolPacket {
            // Word streams carry time but no source-domain sample coordinate.
            // Zero is the documented unavailable-coordinate sentinel.
            start_sample: 0,
            end_sample: 0,
            start_time_ns,
            end_time_ns,
            protocol_id: PACKET_FRAME_PROTOCOL_ID.to_owned(),
            value: ProtocolValue::List(values),
        })
    }

    fn process_word(&mut self, word: Word) -> Vec<ProtocolPacket> {
        if !self.gate_active() {
            return Vec::new();
        }

        let mut packets = Vec::new();
        if self.maximum_gap_ns.is_some_and(|maximum_gap_ns| {
            self.words.last().is_some_and(|previous| {
                word.timestamp_ns.saturating_sub(previous.end_ns()) > maximum_gap_ns
            })
        }) && let Some(packet) = self.close_packet()
        {
            packets.push(packet);
        }

        let delimiter = self
            .delimiter
            .is_some_and(|(delimiter, _)| word.value == delimiter);
        let include_delimiter = self.delimiter.is_some_and(|(_, include)| include);
        if delimiter && !include_delimiter {
            if let Some(packet) = self.close_packet() {
                packets.push(packet);
            }
            return packets;
        }

        self.words.push(word);
        let fixed_complete = self
            .fixed_word_count
            .is_some_and(|count| self.words.len() >= count);
        if (delimiter || fixed_complete || self.words.len() >= self.maximum_words)
            && let Some(packet) = self.close_packet()
        {
            packets.push(packet);
        }
        packets
    }

    fn process_next_event(&mut self) -> Vec<ProtocolPacket> {
        let next = [
            self.gate_head.map(|gate| (gate.start_time_ns, 0_u8)),
            self.boundary_head
                .map(|boundary| (boundary.timestamp_ns, 1_u8)),
            self.word_head
                .as_ref()
                .map(|word| (word.timestamp_ns, 2_u8)),
        ]
        .into_iter()
        .flatten()
        .min();

        match next.map(|(_, kind)| kind) {
            Some(0) => {
                let was_active = self.gate_active();
                self.gate_level = self.gate_head.take().expect("gate head was selected").value;
                if was_active && !self.gate_active() {
                    self.close_packet().into_iter().collect()
                } else {
                    Vec::new()
                }
            }
            Some(1) => {
                self.boundary_head.take();
                self.close_packet().into_iter().collect()
            }
            Some(2) => {
                let word = self.word_head.take().expect("word head was selected");
                self.process_word(word)
            }
            Some(_) => unreachable!(),
            None => Vec::new(),
        }
    }

    fn all_inputs_finished(&self) -> bool {
        self.word_eos
            && (!self.boundary_enabled || self.boundary_eos)
            && (self.gate_polarity.is_none() || self.gate_eos)
            && self.word_head.is_none()
            && self.boundary_head.is_none()
            && self.gate_head.is_none()
    }
}

impl ProcessNode for PacketFramer {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        3
    }

    fn num_outputs(&self) -> usize {
        1
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<Word>("words", 0, PortDirection::Input),
            PortSchema::new::<Trigger>("boundary", 1, PortDirection::Input),
            PortSchema::new::<Sample>("gate", 2, PortDirection::Input),
        ]
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![PortSchema::new::<ProtocolPacket>(
            "packets",
            0,
            PortDirection::Output,
        )]
    }

    fn work_outcome(
        &mut self,
        inputs: &[InputPort],
        outputs: &[OutputPort],
    ) -> WorkResult<WorkOutcome> {
        self.work(inputs, outputs).map(WorkOutcome::progressed)
    }

    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        if self.finished {
            return Err(WorkError::Shutdown);
        }
        let output = outputs
            .first()
            .and_then(|output| output.get::<ProtocolPacket>())
            .ok_or_else(|| WorkError::NodeError("Missing packets output".to_owned()))?;

        self.fill_heads(inputs)?;
        let mut packets = self.process_next_event();
        if self.all_inputs_finished() {
            if let Some(packet) = self.close_packet() {
                packets.push(packet);
            }
            self.finished = true;
        }
        let count = packets.len();
        if count > 0 {
            output.send_batch(packets)?;
        }
        if count == 0 && self.finished {
            Err(WorkError::Shutdown)
        } else {
            Ok(count)
        }
    }
}

fn word_value(word: Word) -> ProtocolValue {
    let mut fields = BTreeMap::from([
        (
            "value".to_owned(),
            ProtocolValue::Integer(i128::from(word.value)),
        ),
        (
            "start_time_ns".to_owned(),
            ProtocolValue::Integer(i128::from(word.timestamp_ns)),
        ),
        (
            "end_time_ns".to_owned(),
            ProtocolValue::Integer(i128::from(word.end_ns())),
        ),
    ]);
    if let Some(payload) = word.payload {
        fields.insert(
            "payload".to_owned(),
            match payload {
                WordPayload::Bytes(bytes) => ProtocolValue::Bytes(bytes),
                WordPayload::Text(text) => ProtocolValue::String(text.to_string()),
            },
        );
    }
    ProtocolValue::Mapping(fields)
}

#[cfg(test)]
mod implementation_tests {
    use crossbeam_channel::{Receiver, Sender as ChannelSender, bounded};
    use signal_processing::{ChannelMessage, Sender, Watchdog};

    use super::*;

    struct Rig {
        word_tx: ChannelSender<ChannelMessage<Word>>,
        boundary_tx: ChannelSender<ChannelMessage<Trigger>>,
        gate_tx: ChannelSender<ChannelMessage<Sample>>,
        inputs: Vec<InputPort>,
        outputs: Vec<OutputPort>,
        packet_rx: Receiver<ChannelMessage<ProtocolPacket>>,
    }

    fn rig() -> Rig {
        let watchdog = Watchdog::new();
        let (word_tx, word_rx) = bounded(128);
        let (boundary_tx, boundary_rx) = bounded(128);
        let (gate_tx, gate_rx) = bounded(128);
        let (packet_tx, packet_rx) = bounded(128);
        Rig {
            word_tx,
            boundary_tx,
            gate_tx,
            inputs: vec![
                InputPort::new_with_watchdog(word_rx, &watchdog, "framer", "words"),
                InputPort::new_with_watchdog(boundary_rx, &watchdog, "framer", "boundary"),
                InputPort::new_with_watchdog(gate_rx, &watchdog, "framer", "gate"),
            ],
            outputs: vec![OutputPort::new_with_watchdog(
                Sender::new(vec![packet_tx]),
                &watchdog,
                "framer",
                "packets",
            )],
            packet_rx,
        }
    }

    fn run(mut framer: PacketFramer, rig: Rig) -> Vec<ProtocolPacket> {
        let Rig {
            word_tx,
            boundary_tx,
            gate_tx,
            inputs,
            outputs,
            packet_rx,
        } = rig;
        drop((word_tx, boundary_tx, gate_tx));
        loop {
            match framer.work(&inputs, &outputs) {
                Ok(_) => {}
                Err(WorkError::Shutdown) => break,
                Err(error) => panic!("unexpected framer error: {error}"),
            }
        }
        packet_rx
            .try_iter()
            .flat_map(|message| match message {
                ChannelMessage::Sample(packet) => vec![packet],
                ChannelMessage::Batch(packets) => packets,
                ChannelMessage::EndOfStream => Vec::new(),
            })
            .collect()
    }

    fn packet_values(packet: &ProtocolPacket) -> Vec<u64> {
        let ProtocolValue::List(words) = &packet.value else {
            panic!("packet value is not a word list");
        };
        words
            .iter()
            .map(|word| {
                let ProtocolValue::Mapping(fields) = word else {
                    panic!("framed word is not a mapping");
                };
                let ProtocolValue::Integer(value) = fields["value"] else {
                    panic!("framed word has no numeric value");
                };
                value as u64
            })
            .collect()
    }

    fn send_words(rig: &Rig, values: &[(u64, u64)]) {
        for &(value, timestamp_ns) in values {
            rig.word_tx
                .send(ChannelMessage::Sample(Word::new(value, timestamp_ns)))
                .unwrap();
        }
    }

    #[test]
    fn fixed_length_frames_and_flushes_the_partial_tail() {
        let rig = rig();
        send_words(&rig, &[(1, 10), (2, 20), (3, 30), (4, 40), (5, 50)]);

        let packets = run(PacketFramer::new().with_fixed_word_count(Some(2)), rig);

        assert_eq!(
            packets.iter().map(packet_values).collect::<Vec<_>>(),
            vec![vec![1, 2], vec![3, 4], vec![5]]
        );
    }

    #[test]
    fn excluded_delimiters_close_but_do_not_enter_packets() {
        let rig = rig();
        send_words(&rig, &[(1, 10), (0xFF, 20), (2, 30), (3, 40), (0xFF, 50)]);

        let packets = run(PacketFramer::new().with_delimiter(Some(0xFF), false), rig);

        assert_eq!(
            packets.iter().map(packet_values).collect::<Vec<_>>(),
            vec![vec![1], vec![2, 3]]
        );
    }

    #[test]
    fn inter_word_gap_closes_before_the_distant_word() {
        let rig = rig();
        send_words(&rig, &[(1, 10), (2, 15), (3, 40)]);

        let packets = run(PacketFramer::new().with_maximum_gap_ns(Some(10)), rig);

        assert_eq!(
            packets.iter().map(packet_values).collect::<Vec<_>>(),
            vec![vec![1, 2], vec![3]]
        );
    }

    #[test]
    fn trigger_boundaries_close_before_same_or_later_words() {
        let rig = rig();
        send_words(&rig, &[(1, 10), (2, 30), (3, 40)]);
        rig.boundary_tx
            .send(ChannelMessage::Sample(Trigger::new(25)))
            .unwrap();
        rig.boundary_tx
            .send(ChannelMessage::Sample(Trigger::new(40)))
            .unwrap();

        let packets = run(PacketFramer::new().with_boundary_input(true), rig);

        assert_eq!(
            packets.iter().map(packet_values).collect::<Vec<_>>(),
            vec![vec![1], vec![2], vec![3]]
        );
    }

    #[test]
    fn gate_activity_frames_words_and_discards_inactive_intervals() {
        let rig = rig();
        send_words(&rig, &[(1, 10), (2, 20), (3, 30), (4, 40)]);
        for sample in [
            Sample::new(false, 0),
            Sample::new(true, 5),
            Sample::new(false, 25),
            Sample::new(true, 35),
            Sample::new(false, 45),
        ] {
            rig.gate_tx.send(ChannelMessage::Sample(sample)).unwrap();
        }

        let packets = run(
            PacketFramer::new().with_gate_input(Some(GatePolarity::ActiveHigh)),
            rig,
        );

        assert_eq!(
            packets.iter().map(packet_values).collect::<Vec<_>>(),
            vec![vec![1, 2], vec![4]]
        );
    }

    #[test]
    fn framed_words_keep_payload_and_time_metadata() {
        let rig = rig();
        rig.word_tx
            .send(ChannelMessage::Sample(Word::labeled(7, "seven", 100, 20)))
            .unwrap();

        let packets = run(PacketFramer::new().with_fixed_word_count(Some(1)), rig);

        assert_eq!(packets[0].start_time_ns, 100);
        assert_eq!(packets[0].end_time_ns, 120);
        let ProtocolValue::List(words) = &packets[0].value else {
            panic!("packet is not a word list");
        };
        let ProtocolValue::Mapping(fields) = &words[0] else {
            panic!("word is not structured");
        };
        assert_eq!(fields["payload"], ProtocolValue::String("seven".to_owned()));
    }
}
