//! Parallel bus decoder for channel-based processing
//!
//! Works with Sample inputs and outputs ParallelWord events.

use super::types::{ParallelWord, StrobeMode, TimingInfo};
use crate::runtime::Receiver;
use crate::runtime::WorkError;
use crate::runtime::node::{InputPort, OutputPort, ProcessNode, WorkResult};
use crate::runtime::sample::Sample;
use std::collections::VecDeque;
use tracing::{debug, trace};

/// Parallel bus decoder node
///
/// Inputs: strobe, data0..dataN, enable (optional) - all Sample channels
/// Output: ParallelWord events
pub struct ParallelDecoder {
    name: String,
    num_data_bits: usize,
    mode: StrobeMode,
    cs_active_low: bool,

    /// Per-channel putback buffers, persisted across work() calls.
    /// Indexed as: strobe=0, data0..dataN=1..N, enable_signal=N+1, cs=N+2
    channel_buffers: Vec<VecDeque<Sample>>,

    last_strobe_value: bool,
    // Debug counter
    work_call_count: usize,
}

impl ParallelDecoder {
    /// Create a new parallel decoder
    ///
    /// # Arguments
    ///
    /// * `num_data_bits` - Number of data bits (1-64)
    /// * `mode` - Strobe trigger mode
    /// * `cs_active_low` - Whether CS is active-low (true) or active-high (false)
    pub fn new(num_data_bits: usize, mode: StrobeMode, cs_active_low: bool) -> Self {
        assert!(
            num_data_bits > 0 && num_data_bits <= 64,
            "Data bits must be 1-64"
        );

        // Channels: strobe + data bits + enable_signal + cs
        let num_channels = 1 + num_data_bits + 2;
        let channel_buffers = (0..num_channels).map(|_| VecDeque::new()).collect();

        Self {
            name: "parallel_decoder".to_string(),
            num_data_bits,
            mode,
            cs_active_low,
            channel_buffers,
            last_strobe_value: false,
            work_call_count: 0,
        }
    }

    /// With custom name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Check if strobe has triggered. Consumes strobe edges until a trigger is found.
    /// Returns the timestamp of the trigger edge if found.
    fn check_strobe_trigger(
        strobe: &mut Receiver<'_, Sample>,
        mode: StrobeMode,
        last_strobe_value: &mut bool,
    ) -> WorkResult<Option<u64>> {
        let edge = strobe.recv()?;

        let triggered = match mode {
            StrobeMode::RisingEdge => !*last_strobe_value && edge.value,
            StrobeMode::FallingEdge => *last_strobe_value && !edge.value,
            StrobeMode::AnyEdge => *last_strobe_value != edge.value,
            StrobeMode::HighLevel => edge.value,
            StrobeMode::LowLevel => !edge.value,
        };

        *last_strobe_value = edge.value;

        if triggered {
            Ok(Some(edge.start_time))
        } else {
            Ok(None)
        }
    }

    /// Read the value of a signal channel at a given timestamp using Sample format.
    fn value_at_time(
        channel: &mut Receiver<'_, Sample>,
        timestamp: u64,
    ) -> WorkResult<Option<bool>> {
        loop {
            let current = channel.recv()?;

            match channel.peek() {
                Ok(next) => {
                    // Check if timestamp is in [current.start_time, next.start_time)
                    if current.start_time <= timestamp && timestamp < next.start_time {
                        channel.put_back(current);
                        return Ok(Some(current.value));
                    }
                    // timestamp >= next.start_time, current has ended - continue
                }
                Err(WorkError::Shutdown) => {
                    // Channel closed - current is the last edge, extends to infinity
                    debug!("Channel peek returned Shutdown at timestamp {}", timestamp);
                    if current.start_time <= timestamp {
                        channel.put_back(current);
                        return Ok(Some(current.value));
                    } else {
                        return Ok(None);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Sample all data bits at a given timestamp
    fn sample_data_at_time(
        data_channels: &mut [Receiver<'_, Sample>],
        timestamp: u64,
    ) -> WorkResult<Option<u64>> {
        let mut value = 0u64;

        for (bit_idx, channel) in data_channels.iter_mut().enumerate() {
            match Self::value_at_time(channel, timestamp)? {
                Some(bit_value) => {
                    if bit_value {
                        value |= 1 << bit_idx;
                    }
                }
                None => {
                    trace!(
                        "Data bit {} not available at timestamp {}",
                        bit_idx, timestamp
                    );
                    return Ok(None);
                }
            }
        }

        Ok(Some(value))
    }

    /// Check if decoder is enabled: enable_signal must be true AND CS must be inactive
    fn check_enabled(
        enable_signal: &mut Receiver<'_, Sample>,
        cs: &mut Receiver<'_, Sample>,
        timestamp: u64,
        cs_active_low: bool,
    ) -> WorkResult<bool> {
        // Get enable signal at timestamp
        let enable_value = match Self::value_at_time(enable_signal, timestamp)? {
            Some(v) => v,
            None => return Ok(false),
        };

        // Get CS value at timestamp
        let cs_value = match Self::value_at_time(cs, timestamp)? {
            Some(v) => v,
            None => return Ok(false),
        };
        // debug!(
        //     "Enable signal: {}, CS value: {} at timestamp {}",
        //     enable_value, cs_value, timestamp
        // );
        // CS inactive logic depends on polarity
        let cs_inactive = if cs_active_low {
            cs_value // CS inactive = high for active-low
        } else {
            !cs_value // CS inactive = low for active-high
        };

        Ok(enable_value && cs_inactive)
    }
}

impl ProcessNode for ParallelDecoder {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        // strobe + data bits + enable_signal + cs
        1 + self.num_data_bits + 2
    }

    fn num_outputs(&self) -> usize {
        1 // ParallelWord output
    }

    fn input_schema(&self) -> Vec<crate::runtime::ports::PortSchema> {
        use crate::Sample;
        use crate::runtime::ports::{PortDirection, PortSchema};

        let mut schemas = vec![PortSchema::new::<Sample>("strobe", 0, PortDirection::Input)];

        // Add data bit inputs
        for i in 0..self.num_data_bits {
            schemas.push(PortSchema::new::<Sample>(
                format!("d{}", i),
                1 + i,
                PortDirection::Input,
            ));
        }

        // Add enable_signal input
        schemas.push(PortSchema::new::<Sample>(
            "enable_signal",
            1 + self.num_data_bits,
            PortDirection::Input,
        ));

        // Add CS input
        schemas.push(PortSchema::new::<Sample>(
            "cs",
            1 + self.num_data_bits + 1,
            PortDirection::Input,
        ));

        schemas
    }

    fn output_schema(&self) -> Vec<crate::runtime::ports::PortSchema> {
        use crate::runtime::ports::{PortDirection, PortSchema};

        vec![PortSchema::new::<ParallelWord>(
            "words",
            0,
            PortDirection::Output,
        )]
    }

    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        self.work_call_count += 1;

        // Debug: Log on first call
        if self.work_call_count == 1 {
            debug!(
                "[{}] First work() call: {} inputs, {} outputs",
                self.name,
                inputs.len(),
                outputs.len()
            );
        }

        // Get output channel with automatic watchdog
        let output = outputs
            .first()
            .and_then(|port| port.get::<ParallelWord>())
            .ok_or_else(|| WorkError::NodeError("Missing output".to_string()))?;

        // Extract config before borrowing channel_buffers
        let mode = self.mode;
        let num_data_bits = self.num_data_bits;
        let cs_active_low = self.cs_active_low;

        // Create Receivers for each channel with automatic watchdog
        let mut buf_iter = self.channel_buffers.iter_mut();
        let mut strobe = inputs
            .first()
            .and_then(|port| port.get::<Sample>(buf_iter.next().unwrap()))
            .ok_or_else(|| WorkError::NodeError("Missing strobe input".to_string()))?;

        // Create data channel receivers
        let mut data_channels: Vec<_> = (0..num_data_bits)
            .map(|i| {
                inputs
                    .get(1 + i)
                    .and_then(|port| port.get::<Sample>(buf_iter.next().unwrap()))
                    .ok_or_else(|| WorkError::NodeError(format!("Missing data input {}", i)))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut enable_signal = inputs
            .get(1 + num_data_bits)
            .and_then(|port| port.get::<Sample>(buf_iter.next().unwrap()))
            .ok_or_else(|| WorkError::NodeError("Missing enable input".to_string()))?;

        let mut cs = inputs
            .get(1 + num_data_bits + 1)
            .and_then(|port| port.get::<Sample>(buf_iter.next().unwrap()))
            .ok_or_else(|| WorkError::NodeError("Missing cs input".to_string()))?;

        let mut words_emitted = 0;

        // Process multiple strobe triggers in one work() call for better throughput
        const MAX_WORDS_PER_CALL: usize = 100;
        while words_emitted < MAX_WORDS_PER_CALL {
            // Check for strobe trigger
            let timestamp =
                match Self::check_strobe_trigger(&mut strobe, mode, &mut self.last_strobe_value)? {
                    Some(ts) => ts,
                    None => continue, // Not a trigger edge, get next strobe edge
                };

            // Check if enabled (enable_signal=true AND CS=inactive)
            if Self::check_enabled(&mut enable_signal, &mut cs, timestamp, cs_active_low)? {
                // Sample data
                if let Some(value) = Self::sample_data_at_time(&mut data_channels, timestamp)? {
                    trace!(
                        "[{}] Decoded word: 0x{:02X} at timestamp {}",
                        self.name, value, timestamp
                    );
                    let word = ParallelWord {
                        value,
                        timing: TimingInfo::new(
                            timestamp as f64 / 1_000.0, // Convert ns to microseconds
                            timestamp,
                        ),
                    };

                    output.send(word)?;
                    words_emitted += 1;
                }
            }
        }

        Ok(words_emitted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_creation() {
        let decoder = ParallelDecoder::new(8, StrobeMode::RisingEdge, true);
        assert_eq!(decoder.num_data_bits, 8);
        assert!(decoder.cs_active_low);
        assert_eq!(decoder.num_inputs(), 11); // strobe + 8 data + enable_signal + cs
    }
}
