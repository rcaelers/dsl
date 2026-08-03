//! Converts qualified signal transitions into trigger events.

use std::collections::VecDeque;

use signal_processing::{
    InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, Sample, Trigger, WorkError,
    WorkOutcome, WorkResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    Rising,
    Falling,
    Both,
}

pub struct EdgeDetector {
    name: String,
    mode: EdgeMode,
    debounce_ns: u64,
    minimum_pulse_width_ns: u64,
    previous: Option<Sample>,
    last_emitted_ns: Option<u64>,
    input_buffer: VecDeque<Sample>,
}

impl EdgeDetector {
    /// Creates an edge detector with the supplied transition policy.
    ///
    /// # Parameters
    /// - `mode`: Input consumed by this operation.
    /// - `debounce_ns`: Input consumed by this operation.
    /// - `minimum_pulse_width_ns`: Input consumed by this operation.
    pub fn new(mode: EdgeMode, debounce_ns: u64, minimum_pulse_width_ns: u64) -> Self {
        Self {
            name: "edge_detector".to_owned(),
            mode,
            debounce_ns,
            minimum_pulse_width_ns,
            previous: None,
            last_emitted_ns: None,
            input_buffer: VecDeque::new(),
        }
    }

    /// Returns this value configured with name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn selected(&self, value: bool) -> bool {
        matches!(
            (self.mode, value),
            (EdgeMode::Rising, true) | (EdgeMode::Falling, false) | (EdgeMode::Both, _)
        )
    }
}

impl ProcessNode for EdgeDetector {
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
        vec![PortSchema::new::<Sample>("signal", 0, PortDirection::Input)]
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![PortSchema::new::<Trigger>(
            "trigger",
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
        let mut input = inputs
            .first()
            .and_then(|port| port.get::<Sample>(&mut self.input_buffer))
            .ok_or_else(|| WorkError::NodeError("Missing signal input".to_owned()))?;
        let output = outputs
            .first()
            .and_then(|port| port.get::<Trigger>())
            .ok_or_else(|| WorkError::NodeError("Missing trigger output".to_owned()))?;
        let sample = input.recv()?;
        let Some(previous) = self.previous else {
            self.previous = Some(sample);
            return Ok(0);
        };
        if sample.value == previous.value {
            return Ok(0);
        }
        self.previous = Some(sample);

        let preceding_width_ns = sample.start_time_ns.saturating_sub(previous.start_time_ns);
        let debounce_elapsed = self
            .last_emitted_ns
            .is_none_or(|last| sample.start_time_ns.saturating_sub(last) >= self.debounce_ns);
        if !self.selected(sample.value)
            || preceding_width_ns < self.minimum_pulse_width_ns
            || !debounce_elapsed
        {
            return Ok(0);
        }

        self.last_emitted_ns = Some(sample.start_time_ns);
        output.send(Trigger::new(sample.start_time_ns))?;
        Ok(1)
    }
}

#[cfg(test)]
mod implementation_tests {
    use crossbeam_channel::bounded;
    use signal_processing::{ChannelMessage, Sender, Watchdog};

    use super::*;

    fn run(detector: &mut EdgeDetector, samples: &[Sample]) -> Vec<u64> {
        let watchdog = Watchdog::new();
        let (input_tx, input_rx) = bounded(64);
        for sample in samples {
            input_tx.send(ChannelMessage::Sample(*sample)).unwrap();
        }
        drop(input_tx);
        let inputs = [InputPort::new_with_watchdog(
            input_rx, &watchdog, "edge", "signal",
        )];
        let (output_tx, output_rx) = bounded::<ChannelMessage<Trigger>>(64);
        let outputs = [OutputPort::new_with_watchdog(
            Sender::new(vec![output_tx]),
            &watchdog,
            "edge",
            "trigger",
        )];
        loop {
            match detector.work(&inputs, &outputs) {
                Ok(_) => {}
                Err(WorkError::Shutdown) => break,
                Err(error) => panic!("unexpected edge detector error: {error}"),
            }
        }
        output_rx
            .try_iter()
            .filter_map(|message| match message {
                ChannelMessage::Sample(trigger) => Some(trigger.timestamp_ns),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn modes_select_the_requested_transitions() {
        let samples = [
            Sample::new(false, 0),
            Sample::new(true, 10),
            Sample::new(false, 20),
            Sample::new(true, 30),
        ];
        assert_eq!(
            run(&mut EdgeDetector::new(EdgeMode::Rising, 0, 0), &samples),
            [10, 30]
        );
        assert_eq!(
            run(&mut EdgeDetector::new(EdgeMode::Falling, 0, 0), &samples),
            [20]
        );
        assert_eq!(
            run(&mut EdgeDetector::new(EdgeMode::Both, 0, 0), &samples),
            [10, 20, 30]
        );
    }

    #[test]
    fn pulse_width_and_debounce_qualify_edges_independently() {
        let samples = [
            Sample::new(false, 0),
            Sample::new(true, 4),
            Sample::new(false, 12),
            Sample::new(true, 20),
            Sample::new(false, 28),
        ];
        assert_eq!(
            run(&mut EdgeDetector::new(EdgeMode::Both, 10, 5), &samples),
            [12, 28]
        );
    }
}
