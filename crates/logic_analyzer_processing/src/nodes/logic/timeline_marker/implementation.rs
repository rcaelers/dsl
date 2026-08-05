//! Processing implementations for persisted graph timeline markers.

use std::collections::VecDeque;

use signal_capture::Sample;
use signal_derived::{TimelineMarker, Trigger};
use signal_runtime::{
    InputPort, OutputPort, PortDirection, PortSchema, ProcessNode, WorkError, WorkOutcome,
    WorkResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerRelation {
    Before,
    AtOrAfter,
}

pub struct TimelineMarkerSource {
    name: String,
    marker: Option<TimelineMarker>,
}

impl TimelineMarkerSource {
    /// Creates a source that emits one marker at a fixed timestamp.
    ///
    /// # Parameters
    /// - `timestamp_ns`: Marker time in nanoseconds.
    pub fn new(timestamp_ns: u64) -> Self {
        Self {
            name: "timeline_marker".into(),
            marker: Some(TimelineMarker::new(timestamp_ns)),
        }
    }

    /// Replaces the runtime name used for diagnostics and graph execution.
    ///
    /// # Parameters
    /// - `name`: New runtime name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl ProcessNode for TimelineMarkerSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        0
    }

    fn num_outputs(&self) -> usize {
        1
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        Vec::new()
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![PortSchema::new::<TimelineMarker>(
            "marker",
            0,
            PortDirection::Output,
        )]
    }

    fn work_outcome(
        &mut self,
        _inputs: &[InputPort],
        outputs: &[OutputPort],
    ) -> WorkResult<WorkOutcome> {
        self.work(&[], outputs).map(WorkOutcome::progressed)
    }

    fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        let Some(marker) = self.marker.take() else {
            return Err(WorkError::Shutdown);
        };
        let output = outputs
            .first()
            .and_then(|port| port.get::<TimelineMarker>())
            .ok_or_else(|| WorkError::NodeError("Missing marker output".into()))?;
        output.send(marker)?;
        Ok(1)
    }
}

pub struct TimelineMarkerToTrigger {
    name: String,
    marker_buffer: VecDeque<TimelineMarker>,
}

impl TimelineMarkerToTrigger {
    /// Creates a converter from timeline markers to trigger events.
    pub fn new() -> Self {
        Self {
            name: "timeline_marker_to_trigger".into(),
            marker_buffer: VecDeque::new(),
        }
    }

    /// Replaces the runtime name used for diagnostics and graph execution.
    ///
    /// # Parameters
    /// - `name`: New runtime name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl Default for TimelineMarkerToTrigger {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessNode for TimelineMarkerToTrigger {
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
        vec![PortSchema::new::<TimelineMarker>(
            "marker",
            0,
            PortDirection::Input,
        )]
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
        let marker = receive_marker(inputs, 0, &mut self.marker_buffer)?;
        let output = outputs
            .first()
            .and_then(|port| port.get::<Trigger>())
            .ok_or_else(|| WorkError::NodeError("Missing trigger output".into()))?;
        output.send(Trigger::new(marker.timestamp_ns))?;
        Ok(1)
    }
}

pub struct TimelineMarkerRelation {
    name: String,
    relation: MarkerRelation,
    marker_buffer: VecDeque<TimelineMarker>,
    pending_samples: VecDeque<Sample>,
    loaded: bool,
}

impl TimelineMarkerRelation {
    /// Creates a relation signal generator for one timeline marker.
    ///
    /// # Parameters
    /// - `relation`: Whether the signal is high before or from the marker timestamp.
    pub fn new(relation: MarkerRelation) -> Self {
        Self {
            name: "timeline_marker_relation".into(),
            relation,
            marker_buffer: VecDeque::new(),
            pending_samples: VecDeque::new(),
            loaded: false,
        }
    }

    /// Replaces the runtime name used for diagnostics and graph execution.
    ///
    /// # Parameters
    /// - `name`: New runtime name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl ProcessNode for TimelineMarkerRelation {
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
        vec![PortSchema::new::<TimelineMarker>(
            "marker",
            0,
            PortDirection::Input,
        )]
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![PortSchema::state::<Sample>(
            "signal",
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
        let output = outputs
            .first()
            .and_then(|port| port.get::<Sample>())
            .ok_or_else(|| WorkError::NodeError("Missing relation signal output".into()))?;
        if let Some(sample) = self.pending_samples.pop_front() {
            output.send(sample)?;
            return Ok(1);
        }
        if self.loaded {
            return Err(WorkError::Shutdown);
        }
        let marker = receive_marker(inputs, 0, &mut self.marker_buffer)?;
        self.pending_samples = relation_samples(self.relation, marker.timestamp_ns).into();
        self.loaded = true;
        let sample = self
            .pending_samples
            .pop_front()
            .expect("every relation has an initial level");
        output.send(sample)?;
        Ok(1)
    }
}

pub struct TimelineMarkerWindow {
    name: String,
    start_buffer: VecDeque<TimelineMarker>,
    end_buffer: VecDeque<TimelineMarker>,
    pending_samples: VecDeque<Sample>,
    loaded: bool,
}

impl TimelineMarkerWindow {
    /// Creates a signal generator that is high between two timeline markers.
    pub fn new() -> Self {
        Self {
            name: "timeline_marker_window".into(),
            start_buffer: VecDeque::new(),
            end_buffer: VecDeque::new(),
            pending_samples: VecDeque::new(),
            loaded: false,
        }
    }

    /// Replaces the runtime name used for diagnostics and graph execution.
    ///
    /// # Parameters
    /// - `name`: New runtime name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl Default for TimelineMarkerWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessNode for TimelineMarkerWindow {
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
            PortSchema::new::<TimelineMarker>("start", 0, PortDirection::Input),
            PortSchema::new::<TimelineMarker>("end", 1, PortDirection::Input),
        ]
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![PortSchema::state::<Sample>(
            "signal",
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
        let output = outputs
            .first()
            .and_then(|port| port.get::<Sample>())
            .ok_or_else(|| WorkError::NodeError("Missing window signal output".into()))?;
        if let Some(sample) = self.pending_samples.pop_front() {
            output.send(sample)?;
            return Ok(1);
        }
        if self.loaded {
            return Err(WorkError::Shutdown);
        }
        let start = receive_marker(inputs, 0, &mut self.start_buffer)?;
        let end = receive_marker(inputs, 1, &mut self.end_buffer)?;
        if start.timestamp_ns > end.timestamp_ns {
            return Err(WorkError::NodeError(
                "Window start marker must not be later than its end marker".into(),
            ));
        }
        self.pending_samples = window_samples(start.timestamp_ns, end.timestamp_ns).into();
        self.loaded = true;
        let sample = self
            .pending_samples
            .pop_front()
            .expect("every marker window has an initial level");
        output.send(sample)?;
        Ok(1)
    }
}

fn receive_marker(
    inputs: &[InputPort],
    index: usize,
    buffer: &mut VecDeque<TimelineMarker>,
) -> WorkResult<TimelineMarker> {
    inputs
        .get(index)
        .and_then(|port| port.get::<TimelineMarker>(buffer))
        .ok_or_else(|| WorkError::NodeError(format!("Missing marker input {index}")))?
        .recv()
}

fn relation_samples(relation: MarkerRelation, timestamp_ns: u64) -> Vec<Sample> {
    match (relation, timestamp_ns) {
        (MarkerRelation::Before, 0) => vec![Sample::new(false, 0)],
        (MarkerRelation::Before, timestamp_ns) => {
            vec![Sample::new(true, 0), Sample::new(false, timestamp_ns)]
        }
        (MarkerRelation::AtOrAfter, 0) => vec![Sample::new(true, 0)],
        (MarkerRelation::AtOrAfter, timestamp_ns) => {
            vec![Sample::new(false, 0), Sample::new(true, timestamp_ns)]
        }
    }
}

fn window_samples(start_ns: u64, end_ns: u64) -> Vec<Sample> {
    if start_ns == end_ns {
        return vec![Sample::new(false, 0)];
    }
    if start_ns == 0 {
        return vec![Sample::new(true, 0), Sample::new(false, end_ns)];
    }
    vec![
        Sample::new(false, 0),
        Sample::new(true, start_ns),
        Sample::new(false, end_ns),
    ]
}

#[cfg(test)]
mod timeline_marker_tests {
    use super::*;

    #[test]
    fn before_and_after_share_an_exact_boundary() {
        assert_eq!(
            relation_samples(MarkerRelation::Before, 25),
            [Sample::new(true, 0), Sample::new(false, 25)]
        );
        assert_eq!(
            relation_samples(MarkerRelation::AtOrAfter, 25),
            [Sample::new(false, 0), Sample::new(true, 25)]
        );
    }

    #[test]
    fn window_is_half_open_and_zero_length_is_empty() {
        assert_eq!(
            window_samples(10, 20),
            [
                Sample::new(false, 0),
                Sample::new(true, 10),
                Sample::new(false, 20),
            ]
        );
        assert_eq!(window_samples(10, 10), [Sample::new(false, 0)]);
    }
}
