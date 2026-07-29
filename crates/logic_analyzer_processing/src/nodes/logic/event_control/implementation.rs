//! Applies timing and optional manual-rearm policy to trigger events.

use std::collections::VecDeque;

use signal_processing::{
    InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, Trigger, WorkError, WorkOutcome,
    WorkResult,
};

pub struct EventControl {
    name: String,
    delay_ns: u64,
    holdoff_ns: u64,
    manual_rearm: bool,
    armed: bool,
    next_allowed_ns: u64,
    event_buffer: VecDeque<Trigger>,
    rearm_buffer: VecDeque<Trigger>,
    event_head: Option<Trigger>,
    rearm_head: Option<Trigger>,
    event_eos: bool,
    rearm_eos: bool,
}

impl EventControl {
    pub fn new(delay_ns: u64, holdoff_ns: u64, manual_rearm: bool) -> Self {
        Self {
            name: "event_control".to_owned(),
            delay_ns,
            holdoff_ns,
            manual_rearm,
            armed: true,
            next_allowed_ns: 0,
            event_buffer: VecDeque::new(),
            rearm_buffer: VecDeque::new(),
            event_head: None,
            rearm_head: None,
            event_eos: false,
            rearm_eos: false,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
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
        if self.manual_rearm && self.rearm_head.is_none() && !self.rearm_eos {
            let mut receiver = inputs
                .get(1)
                .and_then(|port| port.get::<Trigger>(&mut self.rearm_buffer))
                .ok_or_else(|| WorkError::NodeError("Missing rearm input".to_owned()))?;
            match receiver.recv() {
                Ok(rearm) => self.rearm_head = Some(rearm),
                Err(WorkError::Shutdown) => self.rearm_eos = true,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

impl ProcessNode for EventControl {
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
            PortSchema::new::<Trigger>("rearm", 1, PortDirection::Input),
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

        let rearm_precedes_event = self.manual_rearm
            && self.rearm_head.is_some_and(|rearm| {
                self.event_head
                    .is_none_or(|event| rearm.timestamp_ns <= event.timestamp_ns)
            });
        if rearm_precedes_event {
            self.rearm_head.take();
            self.armed = true;
            return Ok(0);
        }

        let event = self.event_head.take().expect("event head was selected");
        let allowed =
            event.timestamp_ns >= self.next_allowed_ns && (!self.manual_rearm || self.armed);
        if !allowed {
            return Ok(0);
        }
        self.next_allowed_ns = event.timestamp_ns.saturating_add(self.holdoff_ns);
        if self.manual_rearm {
            self.armed = false;
        }
        let output = outputs
            .first()
            .and_then(|port| port.get::<Trigger>())
            .ok_or_else(|| WorkError::NodeError("Missing events output".to_owned()))?;
        output.send(Trigger::new(
            event.timestamp_ns.saturating_add(self.delay_ns),
        ))?;
        Ok(1)
    }
}

#[cfg(test)]
mod implementation_tests {
    use crossbeam_channel::{Receiver, Sender as ChannelSender, bounded};
    use signal_processing::{ChannelMessage, Sender, Watchdog};

    use super::*;

    struct Rig {
        event_tx: ChannelSender<ChannelMessage<Trigger>>,
        rearm_tx: ChannelSender<ChannelMessage<Trigger>>,
        inputs: Vec<InputPort>,
        outputs: Vec<OutputPort>,
        output_rx: Receiver<ChannelMessage<Trigger>>,
    }

    fn rig(events: &[u64], rearms: &[u64]) -> Rig {
        let watchdog = Watchdog::new();
        let (event_tx, event_rx) = bounded(64);
        for timestamp_ns in events {
            event_tx
                .send(ChannelMessage::Sample(Trigger::new(*timestamp_ns)))
                .unwrap();
        }
        let (rearm_tx, rearm_rx) = bounded(64);
        for timestamp_ns in rearms {
            rearm_tx
                .send(ChannelMessage::Sample(Trigger::new(*timestamp_ns)))
                .unwrap();
        }
        let (output_tx, output_rx) = bounded(64);
        Rig {
            event_tx,
            rearm_tx,
            inputs: vec![
                InputPort::new_with_watchdog(event_rx, &watchdog, "control", "events"),
                InputPort::new_with_watchdog(rearm_rx, &watchdog, "control", "rearm"),
            ],
            outputs: vec![OutputPort::new_with_watchdog(
                Sender::new(vec![output_tx]),
                &watchdog,
                "control",
                "events",
            )],
            output_rx,
        }
    }

    fn run(mut control: EventControl, rig: Rig) -> Vec<u64> {
        drop((rig.event_tx, rig.rearm_tx));
        loop {
            match control.work(&rig.inputs, &rig.outputs) {
                Ok(_) => {}
                Err(WorkError::Shutdown) => break,
                Err(error) => panic!("unexpected event control error: {error}"),
            }
        }
        rig.output_rx
            .try_iter()
            .filter_map(|message| match message {
                ChannelMessage::Sample(event) => Some(event.timestamp_ns),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn automatic_rearm_applies_holdoff_and_delay() {
        let rig = rig(&[10, 15, 30], &[]);
        assert_eq!(run(EventControl::new(5, 10, false), rig), [15, 35]);
    }

    #[test]
    fn manual_rearm_is_applied_before_an_event_at_the_same_time() {
        let rig = rig(&[10, 20, 30, 40], &[20, 35]);
        assert_eq!(run(EventControl::new(0, 5, true), rig), [10, 20, 40]);
    }
}
