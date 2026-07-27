//! Browser-safe writer stand-ins that consume streams without filesystem I/O.

use std::collections::VecDeque;

use signal_processing::{
    InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, TextSample, Word, WorkError,
    WorkResult,
};

pub struct DiscardWordWriter {
    name: String,
    data: VecDeque<Word>,
    filenames: VecDeque<TextSample>,
}

impl DiscardWordWriter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data: VecDeque::new(),
            filenames: VecDeque::new(),
        }
    }
}

impl ProcessNode for DiscardWordWriter {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        2
    }

    fn num_outputs(&self) -> usize {
        0
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<Word>("data", 0, PortDirection::Input),
            PortSchema::new::<TextSample>("filename", 1, PortDirection::Input),
        ]
    }

    fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
        let mut data = inputs
            .first()
            .and_then(|port| port.get::<Word>(&mut self.data))
            .ok_or_else(|| WorkError::NodeError("Missing data input".to_owned()))?;
        data.recv()?;
        if let Some(mut filenames) = inputs
            .get(1)
            .and_then(|port| port.get::<TextSample>(&mut self.filenames))
        {
            while filenames.try_recv().is_ok() {}
        }
        Ok(1)
    }
}

pub struct DiscardTextWriter {
    name: String,
    lines: VecDeque<TextSample>,
    filenames: VecDeque<TextSample>,
}

impl DiscardTextWriter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            lines: VecDeque::new(),
            filenames: VecDeque::new(),
        }
    }
}

impl ProcessNode for DiscardTextWriter {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        2
    }

    fn num_outputs(&self) -> usize {
        0
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<TextSample>("lines", 0, PortDirection::Input),
            PortSchema::new::<TextSample>("filename", 1, PortDirection::Input),
        ]
    }

    fn work(&mut self, inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
        let mut lines = inputs
            .first()
            .and_then(|port| port.get::<TextSample>(&mut self.lines))
            .ok_or_else(|| WorkError::NodeError("Missing lines input".to_owned()))?;
        lines.recv()?;
        if let Some(mut filenames) = inputs
            .get(1)
            .and_then(|port| port.get::<TextSample>(&mut self.filenames))
        {
            while filenames.try_recv().is_ok() {}
        }
        Ok(1)
    }
}

#[cfg(test)]
mod implementation_tests {
    use crossbeam_channel::bounded;
    use signal_processing::{ChannelMessage, Watchdog};

    use super::*;

    #[test]
    fn word_writer_requires_a_typed_data_input_and_accepts_an_optional_filename_input() {
        let mut missing = DiscardWordWriter::new("discard-words");
        assert!(matches!(
            missing.work(&[], &[]),
            Err(WorkError::NodeError(message)) if message == "Missing data input"
        ));

        let watchdog = Watchdog::new();
        let (sender, receiver) = bounded(1);
        sender
            .send(ChannelMessage::Sample(Word::new(0x42, 10)))
            .unwrap();
        drop(sender);
        let inputs = [InputPort::new_with_watchdog(
            receiver,
            &watchdog,
            "discard-words",
            "data",
        )];

        assert_eq!(missing.work(&inputs, &[]).unwrap(), 1);
        assert!(matches!(
            missing.work(&inputs, &[]),
            Err(WorkError::Shutdown)
        ));
    }

    #[test]
    fn text_writer_requires_a_typed_lines_input_and_drains_filename_levels() {
        let mut missing = DiscardTextWriter::new("discard-text");
        assert!(matches!(
            missing.work(&[], &[]),
            Err(WorkError::NodeError(message)) if message == "Missing lines input"
        ));

        let watchdog = Watchdog::new();
        let (line_sender, line_receiver) = bounded(1);
        let (name_sender, name_receiver) = bounded(2);
        line_sender
            .send(ChannelMessage::Sample(TextSample::new("line", 10)))
            .unwrap();
        name_sender
            .send(ChannelMessage::Sample(TextSample::new("first.txt", 0)))
            .unwrap();
        name_sender
            .send(ChannelMessage::Sample(TextSample::new("second.txt", 5)))
            .unwrap();
        drop(line_sender);
        drop(name_sender);
        let inputs = [
            InputPort::new_with_watchdog(line_receiver, &watchdog, "discard-text", "lines"),
            InputPort::new_with_watchdog(name_receiver, &watchdog, "discard-text", "filename"),
        ];

        assert_eq!(missing.work(&inputs, &[]).unwrap(), 1);
        assert!(missing.filenames.is_empty());
        assert!(matches!(
            missing.work(&inputs, &[]),
            Err(WorkError::Shutdown)
        ));
    }
}
