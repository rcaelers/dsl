//! Filters a trigger stream using a stepped boolean gate level.

use std::collections::VecDeque;

use signal_processing::{Sample, Trigger};
use signal_runtime::{
    InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, WorkError, WorkOutcome,
    WorkResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatePolarity {
    ActiveHigh,
    ActiveLow,
}

pub struct EventGate {
    name: String,
    polarity: GatePolarity,
    gate_level: bool,
    event_buffer: VecDeque<Trigger>,
    gate_buffer: VecDeque<Sample>,
    event_head: Option<Trigger>,
    gate_head: Option<Sample>,
    event_eos: bool,
    gate_eos: bool,
}

impl EventGate {
    /// Creates an event gate with the supplied gating configuration.
    ///
    /// # Parameters
    /// - `polarity`: Input consumed by this operation.
    pub fn new(polarity: GatePolarity) -> Self {
        Self {
            name: "event_gate".to_owned(),
            polarity,
            gate_level: false,
            event_buffer: VecDeque::new(),
            gate_buffer: VecDeque::new(),
            event_head: None,
            gate_head: None,
            event_eos: false,
            gate_eos: false,
        }
    }

    /// Returns this value configured with name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn active(&self) -> bool {
        match self.polarity {
            GatePolarity::ActiveHigh => self.gate_level,
            GatePolarity::ActiveLow => !self.gate_level,
        }
    }

    fn fill_heads(&mut self, inputs: &[InputPort]) -> WorkResult<()> {
        if self.event_head.is_none() && !self.event_eos {
            let mut receiver = inputs
                .first()
                .and_then(|port| port.get::<Trigger>(&mut self.event_buffer))
                .ok_or_else(|| WorkError::NodeError("Missing events input".to_owned()))?;
            match receiver.recv() {
                Ok(event) => self.event_head = Some(event),
                Err(WorkError::Shutdown) => self.event_eos = true,
                Err(error) => return Err(error),
            }
        }
        if self.gate_head.is_none() && !self.gate_eos {
            let mut receiver = inputs
                .get(1)
                .and_then(|port| port.get::<Sample>(&mut self.gate_buffer))
                .ok_or_else(|| WorkError::NodeError("Missing gate input".to_owned()))?;
            match receiver.recv() {
                Ok(gate) => self.gate_head = Some(gate),
                Err(WorkError::Shutdown) => self.gate_eos = true,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl ProcessNode for EventGate {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        2
    }

    fn num_outputs(&self) -> usize {
        1
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<Trigger>("events", 0, PortDirection::Input),
            PortSchema::state::<Sample>("gate", 1, PortDirection::Input),
        ]
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![PortSchema::new::<Trigger>(
            "events",
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
        self.fill_heads(inputs)?;
        if self.event_head.is_none() && self.event_eos {
            return Err(WorkError::Shutdown);
        }

        let gate_precedes_event = self.gate_head.is_some_and(|gate| {
            self.event_head
                .is_none_or(|event| gate.start_time_ns <= event.timestamp_ns)
        });
        if gate_precedes_event {
            self.gate_level = self.gate_head.take().expect("gate head was selected").value;
            return Ok(0);
        }

        let event = self.event_head.take().expect("event head was selected");
        if !self.active() {
            return Ok(0);
        }
        let output = outputs
            .first()
            .and_then(|port| port.get::<Trigger>())
            .ok_or_else(|| WorkError::NodeError("Missing events output".to_owned()))?;
        output.send(event)?;
        Ok(1)
    }
}

#[cfg(test)]
mod implementation_tests {
    use crossbeam_channel::bounded;
    use signal_runtime::{ChannelMessage, Sender, Watchdog};

    use super::*;

    fn run(polarity: GatePolarity) -> Vec<u64> {
        let watchdog = Watchdog::new();
        let (event_tx, event_rx) = bounded(64);
        for timestamp_ns in [10, 20, 30, 40] {
            event_tx
                .send(ChannelMessage::Sample(Trigger::new(timestamp_ns)))
                .unwrap();
        }
        drop(event_tx);
        let (gate_tx, gate_rx) = bounded(64);
        for gate in [
            Sample::new(false, 0),
            Sample::new(true, 15),
            Sample::new(false, 30),
        ] {
            gate_tx.send(ChannelMessage::Sample(gate)).unwrap();
        }
        drop(gate_tx);
        let inputs = [
            InputPort::new_with_watchdog(event_rx, &watchdog, "gate", "events"),
            InputPort::new_with_watchdog(gate_rx, &watchdog, "gate", "gate"),
        ];
        let (output_tx, output_rx) = bounded::<ChannelMessage<Trigger>>(64);
        let outputs = [OutputPort::new_with_watchdog(
            Sender::new(vec![output_tx]),
            &watchdog,
            "gate",
            "events",
        )];
        let mut gate = EventGate::new(polarity);
        loop {
            match gate.work(&inputs, &outputs) {
                Ok(_) => {}
                Err(WorkError::Shutdown) => break,
                Err(error) => panic!("unexpected event gate error: {error}"),
            }
        }
        output_rx
            .try_iter()
            .filter_map(|message| match message {
                ChannelMessage::Sample(event) => Some(event.timestamp_ns),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn gate_changes_apply_before_events_at_the_same_time() {
        assert_eq!(run(GatePolarity::ActiveHigh), [20]);
        assert_eq!(run(GatePolarity::ActiveLow), [10, 30, 40]);
    }
}
