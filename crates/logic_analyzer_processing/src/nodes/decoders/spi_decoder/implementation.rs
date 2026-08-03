//! SPI decoder using an edge-by-edge sequential design.
//!
//! Processes SPI signals one edge at a time using [`Receiver`],
//! which provides peek/putback semantics over a crossbeam channel.
//!
//! Flow per transaction:
//!   1. Wait for CS to go active (read only from CS channel)
//!   2. Wait for CS to go inactive → establishes full CS time window
//!   3. Discard CLK/MOSI/MISO edges from before CS active
//!   4. For each CLK sampling edge within the window, read MOSI/MISO value
//!   5. After bits_per_word bits → emit a `Word` on each configured line's
//!      own output port (MOSI and MISO are independent word streams, not
//!      two fields of one event — a consumer that only cares about one
//!      line wires only that port)
//!   6. Continue collecting words until CLK edges pass the CS window
//!
//! Because each data value is obtained by blocking recv (not try_recv),
//! the race condition from the old batch-decode approach is eliminated.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use tracing::{debug, trace};

use signal_processing::capture::CaptureTransition;
use signal_processing::{
    EdgeQuery, InputPort, OutputPort, PackedSamplingPoint, ProcessNode, ProtocolKind,
    ProtocolPacket, ProtocolValue, Receiver, Sample, SamplingPointStore, Word, WorkError,
    WorkOutcome, WorkResult,
};

use crate::types::{BitOrder, CsPolarity};

pub const SPI_TRANSACTION_PROTOCOL_ID: &str = "org.logicconduit.spi-transaction/v1";

/// SPI clock polarity and phase mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiMode {
    /// CPOL=0, CPHA=0: clock idle low, sample on the rising edge.
    Mode0,
    /// CPOL=0, CPHA=1: clock idle low, sample on the falling edge.
    Mode1,
    /// CPOL=1, CPHA=0: clock idle high, sample on the falling edge.
    Mode2,
    /// CPOL=1, CPHA=1: clock idle high, sample on the rising edge.
    Mode3,
}

/// SPI decoder node
///
/// Inputs: cs, clk, mosi (optional), miso (optional) — Sample channels
/// Outputs: stable MOSI and MISO word, bit-detail, framed-data, and
/// chip-select-bounded transaction streams.
/// A direction without a connected input simply emits no samples on its
/// corresponding outputs.
pub struct SpiDecoder {
    name: String,
    mode: SpiMode,
    bits_per_word: usize,
    has_mosi: bool,
    has_miso: bool,
    cs_polarity: CsPolarity,
    bit_order: BitOrder,

    /// Per-channel putback buffers, persisted across work() calls.
    /// Indexed by CS=0, CLK=1, MOSI=2, MISO=3.
    channel_buffers: Vec<VecDeque<Sample>>,

    /// Tracks CLK state for edge detection across work() boundaries.
    prev_clk: bool,

    /// Transaction counter for logging.
    tx_count: u64,

    /// Query-mode CS search cursor, persisted across work() calls (the
    /// index-query equivalent of the streaming CS `Receiver`'s implicit
    /// position). Unused in streaming mode.
    query_cs_position: u64,
    /// End of the CS-active window currently being decoded. A window can
    /// span several bounded query batches.
    query_window_end: Option<u64>,
    /// Last CLK transition consumed inside the current query window.
    query_clk_position: u64,
    /// Partially assembled query-mode word, retained across bounded batches.
    query_mosi_word: u64,
    query_miso_word: u64,
    query_bits_collected: usize,
    query_first_clock_edge: Option<u64>,
    query_bit_timestamps: Vec<u64>,
    query_mosi_bits: Vec<bool>,
    query_miso_bits: Vec<bool>,
    query_transaction_start: Option<u64>,
    query_transaction_mosi_words: Vec<u64>,
    query_transaction_miso_words: Vec<u64>,
    query_window_deasserted: bool,
    sampling_points: Option<SamplingPointStore>,
}

struct SpiWordAnnotations {
    bits: Vec<Word>,
    data: Word,
}

struct SpiTransaction<'a> {
    start_sample: u64,
    end_sample: u64,
    start_time_ns: u64,
    end_time_ns: u64,
    word_bits: usize,
    mosi_words: &'a [u64],
    miso_words: &'a [u64],
    has_mosi: bool,
    has_miso: bool,
    incomplete_bits: usize,
    cs_deasserted: bool,
}

fn spi_transaction_packet(transaction: SpiTransaction<'_>) -> ProtocolPacket {
    let complete = transaction.incomplete_bits == 0 && transaction.cs_deasserted;
    let mut diagnostics = Vec::new();
    if transaction.incomplete_bits > 0 {
        diagnostics.push(ProtocolValue::String(format!(
            "incomplete word: {}/{} bits",
            transaction.incomplete_bits, transaction.word_bits
        )));
    }
    if !transaction.cs_deasserted {
        diagnostics.push(ProtocolValue::String(
            "chip select remained active at end of capture".to_owned(),
        ));
    }
    let words = |present: bool, values: &[u64]| {
        if present {
            ProtocolValue::List(
                values
                    .iter()
                    .copied()
                    .map(|value| ProtocolValue::Integer(i128::from(value)))
                    .collect(),
            )
        } else {
            ProtocolValue::Null
        }
    };
    ProtocolPacket {
        start_sample: transaction.start_sample,
        end_sample: transaction.end_sample,
        start_time_ns: transaction.start_time_ns,
        end_time_ns: transaction.end_time_ns,
        protocol_id: SPI_TRANSACTION_PROTOCOL_ID.to_owned(),
        value: ProtocolValue::Mapping(BTreeMap::from([
            ("complete".to_owned(), ProtocolValue::Bool(complete)),
            ("diagnostics".to_owned(), ProtocolValue::List(diagnostics)),
            (
                "incomplete_bits".to_owned(),
                ProtocolValue::Integer(transaction.incomplete_bits as i128),
            ),
            (
                "miso".to_owned(),
                words(transaction.has_miso, transaction.miso_words),
            ),
            (
                "mosi".to_owned(),
                words(transaction.has_mosi, transaction.mosi_words),
            ),
            (
                "word_bits".to_owned(),
                ProtocolValue::Integer(transaction.word_bits as i128),
            ),
        ])),
    }
}

fn spi_word_annotations(
    value: u64,
    bit_values: &[bool],
    timestamps: &[u64],
) -> Option<SpiWordAnnotations> {
    if bit_values.len() != timestamps.len() || timestamps.is_empty() {
        return None;
    }

    let cell_start = |index: usize| {
        if timestamps.len() == 1 {
            timestamps[0]
        } else if index == 0 {
            timestamps[0].saturating_sub((timestamps[1] - timestamps[0]) / 2)
        } else {
            timestamps[index - 1] + (timestamps[index] - timestamps[index - 1]) / 2
        }
    };
    let cell_end = |index: usize| {
        if timestamps.len() == 1 {
            timestamps[0].saturating_add(1)
        } else if index + 1 == timestamps.len() {
            let interval = timestamps[index] - timestamps[index - 1];
            timestamps[index].saturating_add(interval.div_ceil(2))
        } else {
            timestamps[index] + (timestamps[index + 1] - timestamps[index]) / 2
        }
    };

    let start = cell_start(0);
    let end = cell_end(timestamps.len() - 1);
    let bits = bit_values
        .iter()
        .enumerate()
        .map(|(index, &bit)| {
            let bit_start = cell_start(index);
            Word::spanning(
                u64::from(bit),
                bit_start,
                cell_end(index).saturating_sub(bit_start).max(1),
            )
        })
        .collect();
    Some(SpiWordAnnotations {
        bits,
        data: Word::spanning(value, start, end.saturating_sub(start).max(1)),
    })
}

impl SpiDecoder {
    /// Create a new SPI decoder with active-low CS (standard)
    ///
    /// # Parameters
    /// - `mode`: Input consumed by this operation.
    /// - `bits_per_word`: Input consumed by this operation.
    /// - `has_mosi`: Input consumed by this operation.
    /// - `has_miso`: Input consumed by this operation.
    pub fn new(mode: SpiMode, bits_per_word: usize, has_mosi: bool, has_miso: bool) -> Self {
        Self::with_cs_polarity(
            mode,
            bits_per_word,
            has_mosi,
            has_miso,
            CsPolarity::ActiveLow,
        )
    }

    /// Create a new SPI decoder with configurable CS polarity
    pub fn with_cs_polarity(
        mode: SpiMode,
        bits_per_word: usize,
        has_mosi: bool,
        has_miso: bool,
        cs_polarity: CsPolarity,
    ) -> Self {
        let num_channels = 2 + usize::from(has_mosi) + usize::from(has_miso);
        Self {
            name: "spi_decoder".to_string(),
            mode,
            bits_per_word,
            has_mosi,
            has_miso,
            cs_polarity,
            bit_order: BitOrder::default(),
            channel_buffers: (0..num_channels).map(|_| VecDeque::new()).collect(),
            prev_clk: false,
            tx_count: 0,
            query_cs_position: 0,
            query_window_end: None,
            query_clk_position: 0,
            query_mosi_word: 0,
            query_miso_word: 0,
            query_bits_collected: 0,
            query_first_clock_edge: None,
            query_bit_timestamps: Vec::with_capacity(bits_per_word),
            query_mosi_bits: Vec::with_capacity(bits_per_word),
            query_miso_bits: Vec::with_capacity(bits_per_word),
            query_transaction_start: None,
            query_transaction_mosi_words: Vec::new(),
            query_transaction_miso_words: Vec::new(),
            query_window_deasserted: false,
            sampling_points: None,
        }
    }

    /// With custom name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// With a bit order other than the default MSB-first
    pub fn with_bit_order(mut self, bit_order: BitOrder) -> Self {
        self.bit_order = bit_order;
        self
    }

    /// Returns this value configured with sampling points.
    pub fn with_sampling_points(mut self, points: SamplingPointStore) -> Self {
        self.sampling_points = Some(points);
        self
    }

    /// Whether the given mode samples on rising CLK edge.
    fn samples_on_rising(&self) -> bool {
        matches!(self.mode, SpiMode::Mode0 | SpiMode::Mode3)
    }

    /// Read the value of a signal channel at a given timestamp.
    ///
    /// With Sample format, an edge is valid from start_time_ns until the
    /// next edge's start_time_ns. We peek at the next edge to determine when
    /// the current edge ends.
    ///
    /// Returns None if the channel is exhausted before finding a valid edge.
    fn value_at_time(
        channel: &mut Receiver<'_, Sample>,
        timestamp: u64,
    ) -> WorkResult<Option<bool>> {
        loop {
            let current = match channel.recv() {
                Ok(edge) => edge,
                Err(WorkError::Shutdown) => {
                    debug!("Channel recv returned Shutdown at timestamp {}", timestamp);
                    return Ok(None);
                }
                Err(e) => return Err(e),
            };

            if current.start_time_ns > timestamp {
                debug!(
                    "value_at_time: edge starts after timestamp ({} > {})",
                    current.start_time_ns, timestamp
                );
            }

            match channel.peek() {
                Ok(next) => {
                    // Check if timestamp is in [current.start_time_ns, next.start_time_ns)
                    if current.start_time_ns <= timestamp && timestamp < next.start_time_ns {
                        channel.put_back(current);
                        return Ok(Some(current.value));
                    }
                    // timestamp >= next.start_time_ns, current has ended - continue
                }
                Err(WorkError::Shutdown) => {
                    // Channel closed - current is the last edge, extends to infinity
                    debug!("Channel peek returned Shutdown at timestamp {}", timestamp);
                    if current.start_time_ns <= timestamp {
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
}

impl ProcessNode for SpiDecoder {
    fn name(&self) -> &str {
        &self.name
    }

    fn num_inputs(&self) -> usize {
        2 + usize::from(self.has_mosi) + usize::from(self.has_miso)
    }

    fn num_outputs(&self) -> usize {
        7
    }

    fn input_schema(&self) -> Vec<signal_processing::PortSchema> {
        use signal_processing::{PortDirection, PortSchema};

        // Every input this decoder has is a raw binary channel: prefer
        // skip-ahead queries over streaming every dead-time edge, fall back
        // to streaming for live sources with no index.
        let protocols = vec![ProtocolKind::EdgeQuery, ProtocolKind::Stream];
        let mut schemas = vec![
            PortSchema::new::<Sample>("cs", 0, PortDirection::Input)
                .with_protocols(protocols.clone()),
            PortSchema::new::<Sample>("clk", 1, PortDirection::Input)
                .with_protocols(protocols.clone()),
        ];
        if self.has_mosi {
            schemas.push(
                PortSchema::new::<Sample>("mosi", 2, PortDirection::Input)
                    .with_protocols(protocols.clone()),
            );
        }
        if self.has_miso {
            let idx = 2 + usize::from(self.has_mosi);
            schemas.push(
                PortSchema::new::<Sample>("miso", idx, PortDirection::Input)
                    .with_protocols(protocols.clone()),
            );
        }
        schemas
    }

    fn output_schema(&self) -> Vec<signal_processing::PortSchema> {
        use signal_processing::{PortDirection, PortSchema};

        // The graph node declares every direction as a stable socket, even
        // when its optional input is disconnected. Keep the runtime port
        // contract equally stable so generic collectors can subscribe to a
        // declared output without needing to understand SPI wiring.
        let mut schemas = [
            "mosi_words",
            "mosi_bits",
            "mosi_data",
            "miso_words",
            "miso_bits",
            "miso_data",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| PortSchema::new::<Word>(name, index, PortDirection::Output))
        .collect::<Vec<_>>();
        schemas.push(PortSchema::new::<ProtocolPacket>(
            "transactions",
            6,
            PortDirection::Output,
        ));
        schemas
    }

    fn work_outcome(
        &mut self,
        inputs: &[InputPort],
        outputs: &[OutputPort],
    ) -> WorkResult<WorkOutcome> {
        match self.work(inputs, outputs) {
            Err(WorkError::Shutdown) => {
                if let Some(points) = &self.sampling_points {
                    points.finish().map_err(|error| {
                        WorkError::NodeError(format!(
                            "could not finish the sampling-point cache: {error}"
                        ))
                    })?;
                }
                Err(WorkError::Shutdown)
            }
            result => result.map(WorkOutcome::progressed),
        }
    }

    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        // Real graphs wire cs/clk/mosi/miso to the same source, so this is
        // an all-or-nothing choice per decoder rather than a per-port mix:
        // if every port this decoder actually uses negotiated EdgeQuery,
        // skip straight to the transactions instead of streaming every
        // sample in between; otherwise stream exactly as before.
        let cs_query = inputs.first().and_then(|p| p.edge_query());
        let clk_query = inputs.get(1).and_then(|p| p.edge_query());
        let mosi_query = if self.has_mosi {
            inputs.get(2).and_then(|p| p.edge_query())
        } else {
            None
        };
        let miso_query = if self.has_miso {
            let idx = 2 + usize::from(self.has_mosi);
            inputs.get(idx).and_then(|p| p.edge_query())
        } else {
            None
        };

        let ready_for_query_mode = cs_query.is_some()
            && clk_query.is_some()
            && (!self.has_mosi || mosi_query.is_some())
            && (!self.has_miso || miso_query.is_some());

        if ready_for_query_mode {
            return self.work_indexed(
                outputs,
                cs_query.expect("checked above"),
                clk_query.expect("checked above"),
                mosi_query,
                miso_query,
            );
        }

        self.work_streamed(inputs, outputs)
    }
}

impl SpiDecoder {
    /// Sampling-edge budget for one index-driven `work()` call. Querying in
    /// batches amortizes index/cache locking while keeping cancellation
    /// responsive for long CS-active windows.
    const QUERY_SAMPLING_EDGES_PER_CALL: usize = 65_536;

    /// Index-driven path: CS locates active transaction windows, CLK edges
    /// are fetched in batches within those windows, and MOSI/MISO values are
    /// fetched in block-grouped batches at the sampling positions. This
    /// avoids both streaming inactive capture regions and taking the shared
    /// index/cache lock separately for every bit.
    fn work_indexed(
        &mut self,
        outputs: &[OutputPort],
        cs_query: Arc<dyn EdgeQuery>,
        clk_query: Arc<dyn EdgeQuery>,
        mosi_query: Option<Arc<dyn EdgeQuery>>,
        miso_query: Option<Arc<dyn EdgeQuery>>,
    ) -> WorkResult<usize> {
        let cs_polarity = self.cs_polarity;
        // Edges alternate, so "wait for active" only ever needs the value
        // that counts as active/inactive, not a predicate — matching
        // `cs_is_active` in `work_streamed` for the two polarities that
        // are actually used in practice. `Disabled` inherits the same
        // degenerate behavior `work_streamed` already has (CS is a
        // mandatory input on this decoder; a polarity that never
        // recognizes "inactive" was already a pre-existing corner case).
        let (active_value, inactive_value) = match cs_polarity {
            CsPolarity::ActiveLow => (false, true),
            CsPolarity::ActiveHigh | CsPolarity::Disabled => (true, false),
        };
        let sample_on_rising = self.samples_on_rising();
        let bits_per_word = self.bits_per_word;
        let bit_order = self.bit_order;
        let bit_position = |bit_index: usize| -> u32 {
            match bit_order {
                BitOrder::MsbFirst => (bits_per_word - 1 - bit_index) as u32,
                BitOrder::LsbFirst => bit_index as u32,
            }
        };
        // A rising-sampling mode looks for the edge landing on `true`
        // (i.e. the rising transition); falling-sampling looks for `false`.
        let sampling_value = sample_on_rising;

        // Each direction owns three adjacent outputs: legacy words, bit
        // detail, and framed data. All are optional when unconnected.
        let mosi_base = 0;
        let miso_base = 3;
        let mosi_output = self
            .has_mosi
            .then(|| outputs.get(mosi_base).and_then(|port| port.get::<Word>()))
            .flatten();
        let mosi_bits_output = self
            .has_mosi
            .then(|| {
                outputs
                    .get(mosi_base + 1)
                    .and_then(|port| port.get::<Word>())
            })
            .flatten();
        let mosi_data_output = self
            .has_mosi
            .then(|| {
                outputs
                    .get(mosi_base + 2)
                    .and_then(|port| port.get::<Word>())
            })
            .flatten();
        let miso_output = self
            .has_miso
            .then(|| outputs.get(miso_base).and_then(|port| port.get::<Word>()))
            .flatten();
        let miso_bits_output = self
            .has_miso
            .then(|| {
                outputs
                    .get(miso_base + 1)
                    .and_then(|port| port.get::<Word>())
            })
            .flatten();
        let miso_data_output = self
            .has_miso
            .then(|| {
                outputs
                    .get(miso_base + 2)
                    .and_then(|port| port.get::<Word>())
            })
            .flatten();
        let need_mosi_annotations = mosi_bits_output.is_some() || mosi_data_output.is_some();
        let need_miso_annotations = miso_bits_output.is_some() || miso_data_output.is_some();
        let need_bit_timestamps = need_mosi_annotations || need_miso_annotations;

        let total_samples = cs_query.total_samples();
        let timestamp_step = (1_000_000_000.0 / cs_query.samplerate_hz()) as u64;
        let position_to_ns = |position: u64| position.saturating_mul(timestamp_step);
        // EdgeQuery methods return signal_processing::Result, not WorkResult.
        let query_err = |e: signal_processing::Error| WorkError::NodeError(e.to_string());

        let mut words_emitted: usize = 0;
        let mut mosi_batch = Vec::new();
        let mut miso_batch = Vec::new();
        let mut mosi_bits_batch = Vec::new();
        let mut mosi_data_batch = Vec::new();
        let mut miso_bits_batch = Vec::new();
        let mut miso_data_batch = Vec::new();
        let mut transaction_batch = Vec::new();
        let mut raw_edges = Vec::<CaptureTransition>::new();
        let mut sample_positions = Vec::<u64>::new();
        let mut mosi_values = Vec::<bool>::new();
        let mut miso_values = Vec::<bool>::new();
        let mut sampling_edges = 0usize;
        let mut capture_exhausted = false;

        while sampling_edges < Self::QUERY_SAMPLING_EDGES_PER_CALL {
            if self.query_window_end.is_none() {
                if self.query_cs_position >= total_samples {
                    capture_exhausted = true;
                    break;
                }

                debug!("Waiting for CS active (query mode)...");
                let cs_active_start = if self.query_cs_position == 0
                    && cs_query.value_at(0).map_err(query_err)? == active_value
                {
                    0
                } else {
                    let Some(edge) = cs_query
                        .next_edge_with_value(self.query_cs_position, active_value, total_samples)
                        .map_err(query_err)?
                    else {
                        capture_exhausted = true;
                        break;
                    };
                    edge.sample
                };
                let inactive_edge = cs_query
                    .next_edge_with_value(cs_active_start, inactive_value, total_samples)
                    .map_err(query_err)?;
                let (inactive_time, cs_deasserted) = inactive_edge
                    .map(|edge| (edge.sample, true))
                    .unwrap_or((total_samples, cs_polarity == CsPolarity::Disabled));

                self.query_window_end = Some(inactive_time);
                self.query_clk_position = cs_active_start;
                self.query_transaction_start = Some(cs_active_start);
                self.query_transaction_mosi_words.clear();
                self.query_transaction_miso_words.clear();
                self.query_window_deasserted = cs_deasserted;
                debug!(
                    "CS window: {:.9}s — {:.9}s ({:.3}µs)",
                    position_to_ns(cs_active_start) as f64 / 1_000_000_000.0,
                    position_to_ns(inactive_time) as f64 / 1_000_000_000.0,
                    (position_to_ns(inactive_time) - position_to_ns(cs_active_start)) as f64
                        / 1_000.0,
                );
            }

            let window_end = self.query_window_end.expect("opened above");
            let remaining = Self::QUERY_SAMPLING_EDGES_PER_CALL - sampling_edges;
            let max_raw_edges = remaining.saturating_mul(2).max(2);
            clk_query
                .next_edges(
                    self.query_clk_position,
                    window_end,
                    max_raw_edges,
                    &mut raw_edges,
                )
                .map_err(query_err)?;
            let window_exhausted = raw_edges.len() < max_raw_edges;
            if let Some(edge) = raw_edges.last() {
                self.query_clk_position = edge.sample;
            }

            sample_positions.clear();
            sample_positions.extend(
                raw_edges
                    .iter()
                    .filter(|edge| edge.value == sampling_value)
                    .map(|edge| edge.sample.saturating_sub(1)),
            );
            sampling_edges += sample_positions.len();

            if let Some(query) = &mosi_query {
                query
                    .values_at(&sample_positions, &mut mosi_values)
                    .map_err(query_err)?;
            } else {
                mosi_values.clear();
            }
            if let Some(query) = &miso_query {
                query
                    .values_at(&sample_positions, &mut miso_values)
                    .map_err(query_err)?;
            } else {
                miso_values.clear();
            }

            let mut sampling_point_batch = self
                .sampling_points
                .as_ref()
                .filter(|store| !store.has_provider())
                .map(|_| Vec::with_capacity(sample_positions.len()));
            for (index, &sample_position) in sample_positions.iter().enumerate() {
                let clock_edge = sample_position.saturating_add(1);
                if self.query_first_clock_edge.is_none() {
                    self.query_first_clock_edge = Some(clock_edge);
                }
                if mosi_query.is_some() && mosi_values[index] {
                    self.query_mosi_word |= 1 << bit_position(self.query_bits_collected);
                }
                if miso_query.is_some() && miso_values[index] {
                    self.query_miso_word |= 1 << bit_position(self.query_bits_collected);
                }
                if need_bit_timestamps {
                    self.query_bit_timestamps.push(position_to_ns(clock_edge));
                }
                if mosi_query.is_some() && need_mosi_annotations {
                    self.query_mosi_bits.push(mosi_values[index]);
                }
                if miso_query.is_some() && need_miso_annotations {
                    self.query_miso_bits.push(miso_values[index]);
                }
                if let Some(points) = &mut sampling_point_batch {
                    let mut values = 0_u64;
                    let mut value_count = 0;
                    if self.has_mosi {
                        values |= u64::from(mosi_values[index]) << value_count;
                        value_count += 1;
                    }
                    if self.has_miso {
                        values |= u64::from(miso_values[index]) << value_count;
                        value_count += 1;
                    }
                    points.push(PackedSamplingPoint::new(
                        position_to_ns(clock_edge),
                        sampling_value,
                        values,
                        value_count,
                    ));
                }
                trace!(
                    "bit {}: CLK edge at {:.9}s, MOSI={}",
                    self.query_bits_collected,
                    position_to_ns(clock_edge) as f64 / 1_000_000_000.0,
                    mosi_query.is_some() && mosi_values[index],
                );

                self.query_bits_collected += 1;
                if self.query_bits_collected < bits_per_word {
                    continue;
                }

                let timestamp = position_to_ns(
                    self.query_first_clock_edge
                        .expect("a complete word has a first edge"),
                );
                let duration = position_to_ns(clock_edge).saturating_sub(timestamp);
                words_emitted += 1;
                debug!(
                    "#{}: mosi=0x{:06X} miso=0x{:06X} at {:.9}s",
                    self.tx_count + words_emitted as u64,
                    self.query_mosi_word,
                    self.query_miso_word,
                    timestamp as f64 / 1_000_000_000.0
                );
                if mosi_output.is_some() {
                    mosi_batch.push(Word::spanning(self.query_mosi_word, timestamp, duration));
                }
                if miso_output.is_some() {
                    miso_batch.push(Word::spanning(self.query_miso_word, timestamp, duration));
                }
                if self.has_mosi {
                    self.query_transaction_mosi_words.push(self.query_mosi_word);
                }
                if self.has_miso {
                    self.query_transaction_miso_words.push(self.query_miso_word);
                }
                if let Some(annotations) = spi_word_annotations(
                    self.query_mosi_word,
                    &self.query_mosi_bits,
                    &self.query_bit_timestamps,
                ) {
                    mosi_bits_batch.extend(annotations.bits);
                    mosi_data_batch.push(annotations.data);
                }
                if let Some(annotations) = spi_word_annotations(
                    self.query_miso_word,
                    &self.query_miso_bits,
                    &self.query_bit_timestamps,
                ) {
                    miso_bits_batch.extend(annotations.bits);
                    miso_data_batch.push(annotations.data);
                }
                self.query_mosi_word = 0;
                self.query_miso_word = 0;
                self.query_bits_collected = 0;
                self.query_first_clock_edge = None;
                self.query_bit_timestamps.clear();
                self.query_mosi_bits.clear();
                self.query_miso_bits.clear();
            }
            if let (Some(store), Some(points)) = (&self.sampling_points, sampling_point_batch) {
                store.record_packed_batch(points).map_err(|error| {
                    WorkError::NodeError(format!("could not cache SPI sampling points: {error}"))
                })?;
            }

            if window_exhausted {
                let incomplete_bits = self.query_bits_collected;
                if self.query_bits_collected > 0 {
                    debug!(
                        "Incomplete word: {}/{} bits",
                        self.query_bits_collected, bits_per_word
                    );
                }
                let transaction_start = self
                    .query_transaction_start
                    .take()
                    .expect("an open query window has a transaction start");
                transaction_batch.push(spi_transaction_packet(SpiTransaction {
                    start_sample: transaction_start,
                    end_sample: window_end,
                    start_time_ns: position_to_ns(transaction_start),
                    end_time_ns: position_to_ns(window_end),
                    word_bits: bits_per_word,
                    mosi_words: &self.query_transaction_mosi_words,
                    miso_words: &self.query_transaction_miso_words,
                    has_mosi: self.has_mosi,
                    has_miso: self.has_miso,
                    incomplete_bits,
                    cs_deasserted: self.query_window_deasserted,
                }));
                self.query_transaction_mosi_words.clear();
                self.query_transaction_miso_words.clear();
                self.query_mosi_word = 0;
                self.query_miso_word = 0;
                self.query_bits_collected = 0;
                self.query_first_clock_edge = None;
                self.query_bit_timestamps.clear();
                self.query_mosi_bits.clear();
                self.query_miso_bits.clear();
                self.query_cs_position = window_end;
                self.query_window_end = None;
            }
        }

        if let Some(output) = mosi_output {
            output.send_batch(mosi_batch)?;
        }
        if let Some(output) = miso_output {
            output.send_batch(miso_batch)?;
        }
        if let Some(output) = mosi_bits_output {
            output.send_batch(mosi_bits_batch)?;
        }
        if let Some(output) = mosi_data_output {
            output.send_batch(mosi_data_batch)?;
        }
        if let Some(output) = miso_bits_output {
            output.send_batch(miso_bits_batch)?;
        }
        if let Some(output) = miso_data_output {
            output.send_batch(miso_data_batch)?;
        }
        let transactions_emitted = transaction_batch.len();
        if let Some(output) = outputs.get(6).and_then(|port| port.get::<ProtocolPacket>()) {
            output.send_batch(transaction_batch)?;
        }

        self.tx_count += words_emitted as u64;
        if capture_exhausted {
            Err(WorkError::Shutdown)
        } else {
            Ok(words_emitted.max(transactions_emitted))
        }
    }

    /// Streaming path: unchanged behavior for live sources or any
    /// connection that didn't negotiate `EdgeQuery`.
    fn work_streamed(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        // Extract config values before borrowing channel_buffers
        let cs_polarity = self.cs_polarity;
        let cs_is_active = |value: bool| -> bool {
            match cs_polarity {
                CsPolarity::ActiveLow => !value,
                CsPolarity::ActiveHigh => value,
                CsPolarity::Disabled => true,
            }
        };
        let sample_on_rising = self.samples_on_rising();
        let bits_per_word = self.bits_per_word;
        let has_mosi = self.has_mosi;
        let has_miso = self.has_miso;
        let bit_order = self.bit_order;
        let bit_position = |bit_index: usize| -> u32 {
            match bit_order {
                BitOrder::MsbFirst => (bits_per_word - 1 - bit_index) as u32,
                BitOrder::LsbFirst => bit_index as u32,
            }
        };
        let mut prev_clk = self.prev_clk;

        // ── Create named Receivers per channel with automatic watchdog ───────
        let mut buf_iter = self.channel_buffers.iter_mut();
        let mut cs = inputs
            .first()
            .and_then(|p| p.get::<Sample>(buf_iter.next().unwrap()))
            .ok_or_else(|| WorkError::NodeError("Missing CS input".into()))?;
        let mut clk = inputs
            .get(1)
            .and_then(|p| p.get::<Sample>(buf_iter.next().unwrap()))
            .ok_or_else(|| WorkError::NodeError("Missing CLK input".into()))?;
        let mut mosi = if has_mosi {
            Some(
                inputs
                    .get(2)
                    .and_then(|p| p.get::<Sample>(buf_iter.next().unwrap()))
                    .ok_or_else(|| WorkError::NodeError("Missing MOSI input".into()))?,
            )
        } else {
            None
        };
        let mut miso = if has_miso {
            let idx = 2 + usize::from(has_mosi);
            Some(
                inputs
                    .get(idx)
                    .and_then(|p| p.get::<Sample>(buf_iter.next().unwrap()))
                    .ok_or_else(|| WorkError::NodeError("Missing MISO input".into()))?,
            )
        } else {
            None
        };
        let mosi_base = 0;
        let miso_base = 3;
        let mosi_output = has_mosi
            .then(|| outputs.get(mosi_base).and_then(|port| port.get::<Word>()))
            .flatten();
        let mosi_bits_output = has_mosi
            .then(|| {
                outputs
                    .get(mosi_base + 1)
                    .and_then(|port| port.get::<Word>())
            })
            .flatten();
        let mosi_data_output = has_mosi
            .then(|| {
                outputs
                    .get(mosi_base + 2)
                    .and_then(|port| port.get::<Word>())
            })
            .flatten();
        let miso_output = has_miso
            .then(|| outputs.get(miso_base).and_then(|port| port.get::<Word>()))
            .flatten();
        let miso_bits_output = has_miso
            .then(|| {
                outputs
                    .get(miso_base + 1)
                    .and_then(|port| port.get::<Word>())
            })
            .flatten();
        let miso_data_output = has_miso
            .then(|| {
                outputs
                    .get(miso_base + 2)
                    .and_then(|port| port.get::<Word>())
            })
            .flatten();
        let need_mosi_annotations = mosi_bits_output.is_some() || mosi_data_output.is_some();
        let need_miso_annotations = miso_bits_output.is_some() || miso_data_output.is_some();
        let need_bit_timestamps = need_mosi_annotations || need_miso_annotations;

        // ── 1. Wait for CS to go active ──────────────────────────────────
        // Only read from CS — leave CLK/MOSI/MISO untouched so their edges
        // (which may span the CS-active window) aren't lost.
        debug!("Waiting for CS active...");
        let cs_active_edge = loop {
            let edge = cs.recv()?;
            if cs_is_active(edge.value) {
                break edge;
            }
        };

        let cs_active_start = cs_active_edge.start_time_ns;

        // ── 2. Get CS inactive edge to know the full CS window ───────────
        let mut cs_terminal_time = cs_active_start;
        let (cs_inactive_time, cs_deasserted) = loop {
            match cs.recv() {
                Ok(edge) => {
                    cs_terminal_time = edge.start_time_ns;
                    if !cs_is_active(edge.value) {
                        break (edge.start_time_ns, true);
                    }
                }
                // Capture-backed edge streams terminate with a same-level
                // sample at the capture end. If CS is still active there,
                // use that terminal sample as the bounded end of the final
                // transaction so its complete words are not discarded.
                Err(WorkError::Shutdown) if cs_terminal_time > cs_active_start => {
                    break (cs_terminal_time, cs_polarity == CsPolarity::Disabled);
                }
                Err(error) => return Err(error),
            }
        };
        debug!(
            "CS window: {:.9}s — {:.9}s ({:.3}µs)",
            cs_active_start as f64 / 1_000_000_000.0,
            cs_inactive_time as f64 / 1_000_000_000.0,
            (cs_inactive_time - cs_active_start) as f64 / 1_000.0,
        );

        // ── 3. Discard CLK/MOSI/MISO edges from before CS active ────────
        clk.drain_before(cs_active_start, |e| e.start_time_ns)?;
        if let Some(ref mut m) = mosi {
            m.drain_before(cs_active_start, |e| e.start_time_ns)?;
        }
        if let Some(ref mut m) = miso {
            m.drain_before(cs_active_start, |e| e.start_time_ns)?;
        }

        // ── 4. Collect words from CLK within the CS window ───────────────
        let mut words_emitted: usize = 0;
        let mut mosi_batch = Vec::new();
        let mut miso_batch = Vec::new();
        let mut mosi_bits_batch = Vec::new();
        let mut mosi_data_batch = Vec::new();
        let mut miso_bits_batch = Vec::new();
        let mut miso_data_batch = Vec::new();
        let mut transaction_mosi_words = Vec::new();
        let mut transaction_miso_words = Vec::new();
        let mut sampling_point_batch = self
            .sampling_points
            .as_ref()
            .filter(|store| !store.has_provider())
            .map(|_| Vec::new());
        let mut incomplete_bits = 0usize;

        'word_loop: loop {
            let mut mosi_word: u64 = 0;
            let mut miso_word: u64 = 0;
            let mut bits_collected: usize = 0;
            // The word's (first, last) sampling-edge timestamps so far.
            let mut clock_edge_span: Option<(u64, u64)> = None;
            let mut bit_timestamps = Vec::with_capacity(bits_per_word);
            let mut mosi_bits = Vec::with_capacity(bits_per_word);
            let mut miso_bits = Vec::with_capacity(bits_per_word);

            // Collect bits_per_word bits from CLK sampling edges.
            // MOSI/MISO are read on-demand via value_at_time when a
            // CLK sampling edge arrives. CS is already fully consumed;
            // we use cs_inactive_time for bounds.
            loop {
                let edge = match clk.recv() {
                    Ok(edge) => edge,
                    Err(WorkError::Shutdown) => {
                        incomplete_bits = bits_collected;
                        break 'word_loop;
                    }
                    Err(error) => return Err(error),
                };

                // CLK edge past CS window → transaction is over
                if edge.start_time_ns >= cs_inactive_time {
                    clk.put_back(edge);

                    if bits_collected > 0 && bits_collected < bits_per_word {
                        debug!("Incomplete word: {}/{} bits", bits_collected, bits_per_word);
                        incomplete_bits = bits_collected;
                    }

                    break 'word_loop;
                }

                // Check for sampling edge
                let is_rising = !prev_clk && edge.value;
                let is_falling = prev_clk && !edge.value;
                prev_clk = edge.value;

                let is_sampling_edge = if sample_on_rising {
                    is_rising
                } else {
                    is_falling
                };

                if !is_sampling_edge {
                    continue;
                }

                // Record first/last clock edge timestamps
                clock_edge_span = Some(match clock_edge_span {
                    None => (edge.start_time_ns, edge.start_time_ns),
                    Some((first, _)) => (first, edge.start_time_ns),
                });

                // Sample data lines at CLK edge time
                let sample_time = edge.start_time_ns.saturating_sub(1);
                let mut sampled_values = 0_u64;
                let mut sampled_value_count = 0;
                if has_mosi {
                    match Self::value_at_time(mosi.as_mut().unwrap(), sample_time)? {
                        Some(mosi_val) => {
                            if mosi_val {
                                mosi_word |= 1 << bit_position(bits_collected);
                            }
                            trace!(
                                "bit {}: CLK edge at {:.9}s, MOSI={}",
                                bits_collected,
                                edge.start_time_ns as f64 / 1_000_000_000.0,
                                mosi_val,
                            );
                            if need_mosi_annotations {
                                mosi_bits.push(mosi_val);
                            }
                            sampled_values |= u64::from(mosi_val) << sampled_value_count;
                            sampled_value_count += 1;
                        }
                        None => {
                            // MOSI channel exhausted - signal shutdown
                            debug!("MOSI channel exhausted, shutting down decoder");
                            return Err(WorkError::Shutdown);
                        }
                    }
                }

                if has_miso {
                    match Self::value_at_time(miso.as_mut().unwrap(), sample_time)? {
                        Some(miso_val) => {
                            if miso_val {
                                miso_word |= 1 << bit_position(bits_collected);
                            }
                            if need_miso_annotations {
                                miso_bits.push(miso_val);
                            }
                            sampled_values |= u64::from(miso_val) << sampled_value_count;
                            sampled_value_count += 1;
                        }
                        None => {
                            // MISO channel exhausted - signal shutdown
                            debug!("MISO channel exhausted, shutting down decoder");
                            return Err(WorkError::Shutdown);
                        }
                    }
                }

                if need_bit_timestamps {
                    bit_timestamps.push(edge.start_time_ns);
                }
                if let Some(points) = &mut sampling_point_batch {
                    points.push(PackedSamplingPoint::new(
                        edge.start_time_ns,
                        edge.value,
                        sampled_values,
                        sampled_value_count,
                    ));
                }
                bits_collected += 1;
                if bits_collected >= bits_per_word {
                    break;
                }
            }

            // We have a complete word
            if bits_collected == bits_per_word {
                // First to last sampling edge — the word's real extent.
                let (timestamp, last_edge) =
                    clock_edge_span.unwrap_or((cs_active_start, cs_active_start));
                let duration = last_edge.saturating_sub(timestamp);

                words_emitted += 1;
                debug!(
                    "#{}: mosi=0x{:06X} miso=0x{:06X} at {:.9}s",
                    self.tx_count + words_emitted as u64,
                    mosi_word,
                    miso_word,
                    timestamp as f64 / 1_000_000_000.0
                );
                if mosi_output.is_some() {
                    mosi_batch.push(Word::spanning(mosi_word, timestamp, duration));
                }
                if miso_output.is_some() {
                    miso_batch.push(Word::spanning(miso_word, timestamp, duration));
                }
                if has_mosi {
                    transaction_mosi_words.push(mosi_word);
                }
                if has_miso {
                    transaction_miso_words.push(miso_word);
                }
                if let Some(annotations) =
                    spi_word_annotations(mosi_word, &mosi_bits, &bit_timestamps)
                {
                    mosi_bits_batch.extend(annotations.bits);
                    mosi_data_batch.push(annotations.data);
                }
                if let Some(annotations) =
                    spi_word_annotations(miso_word, &miso_bits, &bit_timestamps)
                {
                    miso_bits_batch.extend(annotations.bits);
                    miso_data_batch.push(annotations.data);
                }
            }
        }

        if let (Some(store), Some(points)) = (&self.sampling_points, sampling_point_batch) {
            store.record_packed_batch(points).map_err(|error| {
                WorkError::NodeError(format!("could not cache SPI sampling points: {error}"))
            })?;
        }

        if let Some(output) = mosi_output {
            output.send_batch(mosi_batch)?;
        }
        if let Some(output) = miso_output {
            output.send_batch(miso_batch)?;
        }
        if let Some(output) = mosi_bits_output {
            output.send_batch(mosi_bits_batch)?;
        }
        if let Some(output) = mosi_data_output {
            output.send_batch(mosi_data_batch)?;
        }
        if let Some(output) = miso_bits_output {
            output.send_batch(miso_bits_batch)?;
        }
        if let Some(output) = miso_data_output {
            output.send_batch(miso_data_batch)?;
        }
        let transaction = spi_transaction_packet(SpiTransaction {
            start_sample: 0,
            end_sample: 0,
            start_time_ns: cs_active_start,
            end_time_ns: cs_inactive_time,
            word_bits: bits_per_word,
            mosi_words: &transaction_mosi_words,
            miso_words: &transaction_miso_words,
            has_mosi,
            has_miso,
            incomplete_bits,
            cs_deasserted,
        });
        if let Some(output) = outputs.get(6).and_then(|port| port.get::<ProtocolPacket>()) {
            output.send(transaction)?;
        }

        // Write back mutable state
        self.prev_clk = prev_clk;
        self.tx_count += words_emitted as u64;

        Ok(words_emitted.max(1))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct QueryCalls {
        next_edge: AtomicUsize,
        next_edges: AtomicUsize,
        value_at: AtomicUsize,
        values_at: AtomicUsize,
    }

    struct BatchQuery {
        bits: Vec<bool>,
        calls: Arc<QueryCalls>,
    }

    impl BatchQuery {
        fn transition_after(&self, position: u64, limit: u64) -> Option<CaptureTransition> {
            let mut current = self.bits[position as usize];
            for sample in position + 1..limit.min(self.bits.len() as u64) {
                let value = self.bits[sample as usize];
                if value != current {
                    return Some(CaptureTransition { sample, value });
                }
                current = value;
            }
            None
        }
    }

    impl EdgeQuery for BatchQuery {
        fn sample_period(&self) -> f64 {
            1e-9
        }

        fn samplerate_hz(&self) -> f64 {
            1e9
        }

        fn total_samples(&self) -> u64 {
            self.bits.len() as u64
        }

        fn value_at(&self, position: u64) -> signal_processing::Result<bool> {
            self.calls.value_at.fetch_add(1, Ordering::Relaxed);
            Ok(self.bits[position as usize])
        }

        fn next_edge(
            &self,
            position: u64,
            limit: u64,
        ) -> signal_processing::Result<Option<CaptureTransition>> {
            self.calls.next_edge.fetch_add(1, Ordering::Relaxed);
            Ok(self.transition_after(position, limit))
        }

        fn next_edges(
            &self,
            position: u64,
            limit: u64,
            max_edges: usize,
            output: &mut Vec<CaptureTransition>,
        ) -> signal_processing::Result<()> {
            self.calls.next_edges.fetch_add(1, Ordering::Relaxed);
            output.clear();
            let mut cursor = position;
            while output.len() < max_edges {
                let Some(edge) = self.transition_after(cursor, limit) else {
                    break;
                };
                cursor = edge.sample;
                output.push(edge);
            }
            Ok(())
        }

        fn values_at(
            &self,
            positions: &[u64],
            output: &mut Vec<bool>,
        ) -> signal_processing::Result<()> {
            self.calls.values_at.fetch_add(1, Ordering::Relaxed);
            output.clear();
            output.extend(
                positions
                    .iter()
                    .map(|&position| self.bits[position as usize]),
            );
            Ok(())
        }
    }

    fn query_input(
        watchdog: &signal_processing::Watchdog,
        bits: Vec<bool>,
        calls: Arc<QueryCalls>,
        port: &str,
    ) -> InputPort {
        InputPort::disconnected()
            .with_edge_query(Some(Arc::new(BatchQuery { bits, calls })))
            .with_watchdog(watchdog.clone(), "spi".to_string(), port.to_string())
    }

    #[test]
    fn test_decoder_creation() {
        let decoder = SpiDecoder::new(SpiMode::Mode0, 8, true, false);
        assert_eq!(decoder.bits_per_word, 8);
        assert!(decoder.has_mosi);
        assert!(!decoder.has_miso);
        assert_eq!(decoder.channel_buffers.len(), 3); // CS, CLK, MOSI
    }

    #[test]
    fn optional_miso_keeps_the_declared_output_port_contract() {
        let decoder = SpiDecoder::new(SpiMode::Mode0, 8, true, false);

        assert_eq!(decoder.num_outputs(), 7);
        assert_eq!(
            decoder
                .output_schema()
                .into_iter()
                .map(|schema| schema.name)
                .collect::<Vec<_>>(),
            [
                "mosi_words",
                "mosi_bits",
                "mosi_data",
                "miso_words",
                "miso_bits",
                "miso_data",
                "transactions",
            ]
        );
    }

    #[test]
    fn test_decoder_creation_with_miso() {
        let decoder = SpiDecoder::new(SpiMode::Mode0, 24, true, true);
        assert_eq!(decoder.bits_per_word, 24);
        assert!(decoder.has_mosi);
        assert!(decoder.has_miso);
        assert_eq!(decoder.channel_buffers.len(), 4); // CS, CLK, MOSI, MISO
    }

    #[test]
    fn test_cs_polarity() {
        let decoder_low = SpiDecoder::new(SpiMode::Mode0, 8, true, false);
        assert_eq!(decoder_low.cs_polarity, CsPolarity::ActiveLow);

        let decoder_high =
            SpiDecoder::with_cs_polarity(SpiMode::Mode0, 8, true, false, CsPolarity::ActiveHigh);
        assert_eq!(decoder_high.cs_polarity, CsPolarity::ActiveHigh);
    }

    #[test]
    fn incomplete_transaction_reports_partial_word_and_active_cs() {
        let packet = spi_transaction_packet(SpiTransaction {
            start_sample: 10,
            end_sample: 20,
            start_time_ns: 100,
            end_time_ns: 200,
            word_bits: 8,
            mosi_words: &[0x12],
            miso_words: &[],
            has_mosi: true,
            has_miso: false,
            incomplete_bits: 3,
            cs_deasserted: false,
        });

        let ProtocolValue::Mapping(fields) = packet.value else {
            panic!("transaction value should be a mapping");
        };
        assert_eq!(fields["complete"], ProtocolValue::Bool(false));
        assert_eq!(fields["incomplete_bits"], ProtocolValue::Integer(3));
        assert_eq!(fields["miso"], ProtocolValue::Null);
        assert_eq!(
            fields["diagnostics"],
            ProtocolValue::List(vec![
                ProtocolValue::String("incomplete word: 3/8 bits".to_owned()),
                ProtocolValue::String("chip select remained active at end of capture".to_owned()),
            ])
        );
    }

    #[test]
    fn work_indexed_batches_clock_edges_and_data_values() {
        use crossbeam_channel::bounded;
        use signal_processing::{ChannelMessage, Sender, Watchdog};

        let samples = 100usize;
        let cs_bits = (0..samples)
            .map(|sample| !(10..90).contains(&sample))
            .collect();
        let clk_bits = (0..samples).map(|sample| sample % 2 == 1).collect();
        let mosi_bits = vec![true; samples];
        let cs_calls = Arc::new(QueryCalls::default());
        let clk_calls = Arc::new(QueryCalls::default());
        let mosi_calls = Arc::new(QueryCalls::default());
        let watchdog = Watchdog::new();
        let inputs = [
            query_input(&watchdog, cs_bits, cs_calls, "cs"),
            query_input(&watchdog, clk_bits, Arc::clone(&clk_calls), "clk"),
            query_input(&watchdog, mosi_bits, Arc::clone(&mosi_calls), "mosi"),
        ];
        let (words_tx, words_rx) = bounded::<ChannelMessage<Word>>(4);
        let (bits_tx, bits_rx) = bounded::<ChannelMessage<Word>>(4);
        let (data_tx, data_rx) = bounded::<ChannelMessage<Word>>(4);
        let outputs = [
            OutputPort::new_with_watchdog(
                Sender::new(vec![words_tx]),
                &watchdog,
                "spi",
                "mosi_words",
            ),
            OutputPort::new_with_watchdog(
                Sender::new(vec![bits_tx]),
                &watchdog,
                "spi",
                "mosi_bits",
            ),
            OutputPort::new_with_watchdog(
                Sender::new(vec![data_tx]),
                &watchdog,
                "spi",
                "mosi_data",
            ),
        ];
        let sampling_points = SamplingPointStore::default();
        let mut decoder = SpiDecoder::new(SpiMode::Mode0, 4, true, false)
            .with_sampling_points(sampling_points.clone());

        assert!(matches!(
            decoder.work(&inputs, &outputs),
            Err(WorkError::Shutdown)
        ));
        let collect = |rx: crossbeam_channel::Receiver<ChannelMessage<Word>>| -> Vec<Word> {
            rx.try_iter()
                .flat_map(|message| match message {
                    ChannelMessage::Sample(word) => vec![word],
                    ChannelMessage::Batch(words) => words,
                    ChannelMessage::EndOfStream => vec![],
                })
                .collect()
        };
        let words = collect(words_rx);
        let bits = collect(bits_rx);
        let data = collect(data_rx);

        assert_eq!(words.len(), 10);
        assert!(words.iter().all(|word| word.value == 0b1111));
        assert_eq!(words[0], Word::spanning(0b1111, 11, 6));
        assert_eq!(bits.len(), 40);
        assert_eq!(
            &bits[..4],
            &[
                Word::spanning(1, 10, 2),
                Word::spanning(1, 12, 2),
                Word::spanning(1, 14, 2),
                Word::spanning(1, 16, 2),
            ]
        );
        assert_eq!(data[0], Word::spanning(0b1111, 10, 8));
        assert!(clk_calls.next_edges.load(Ordering::Relaxed) > 0);
        assert_eq!(clk_calls.next_edge.load(Ordering::Relaxed), 0);
        assert!(mosi_calls.values_at.load(Ordering::Relaxed) > 0);
        assert_eq!(mosi_calls.value_at.load(Ordering::Relaxed), 0);
        let sampling_points = sampling_points.points_in_range(0, u64::MAX);
        assert_eq!(sampling_points.len(), 40);
        assert_eq!(sampling_points.first().unwrap().time_ns, 11);
        assert_eq!(sampling_points.last().unwrap().time_ns, 89);
        assert!(sampling_points.iter().all(|point| point.clock_high));
        assert!(sampling_points.iter().all(|point| point.values == [true]));
    }

    /// MOSI and MISO are independent `Word` streams on independent output
    /// ports — the regression test for the bug this design fixes (the old
    /// single-`SpiTransfer`-port design could never actually deliver MISO
    /// to a consumer; see `docs/PIPELINE_DESIGN.md` and this file's
    /// module doc). 4-bit words, MSB-first: MOSI = 0b1010, MISO = 0b0101,
    /// sampled on CLK's rising edge (Mode0).
    #[test]
    fn work_streamed_emits_independent_mosi_and_miso_word_streams() {
        use crossbeam_channel::bounded;
        use signal_processing::{ChannelMessage, Sender, Watchdog};

        let wd = Watchdog::new();

        let cs_samples = [Sample::new(false, 0), Sample::new(true, 1000)];
        let clk_samples = [
            Sample::new(false, 0),
            Sample::new(true, 100),
            Sample::new(false, 200),
            Sample::new(true, 300),
            Sample::new(false, 400),
            Sample::new(true, 500),
            Sample::new(false, 600),
            Sample::new(true, 700),
        ];
        // Bit i is read 1ns before CLK's i-th rising edge.
        let mosi_samples = [
            Sample::new(true, 0),    // bit0 = 1
            Sample::new(false, 200), // bit1 = 0
            Sample::new(true, 400),  // bit2 = 1
            Sample::new(false, 600), // bit3 = 0
        ];
        let miso_samples = [
            Sample::new(false, 0),   // bit0 = 0
            Sample::new(true, 200),  // bit1 = 1
            Sample::new(false, 400), // bit2 = 0
            Sample::new(true, 600),  // bit3 = 1
        ];

        let make_input = |samples: &[Sample], port: &str| {
            let (tx, rx) = bounded::<ChannelMessage<Sample>>(samples.len() + 1);
            for &s in samples {
                tx.send(ChannelMessage::Sample(s)).unwrap();
            }
            drop(tx);
            InputPort::new_with_watchdog(rx, &wd, "spi", port)
        };
        let inputs = [
            make_input(&cs_samples, "cs"),
            make_input(&clk_samples, "clk"),
            make_input(&mosi_samples, "mosi"),
            make_input(&miso_samples, "miso"),
        ];

        let output = || {
            let (tx, rx) = bounded::<ChannelMessage<Word>>(16);
            (Sender::new(vec![tx]), rx)
        };
        let (mosi_words, mosi_words_rx) = output();
        let (mosi_bits, mosi_bits_rx) = output();
        let (mosi_data, mosi_data_rx) = output();
        let (miso_words, miso_words_rx) = output();
        let (miso_bits, miso_bits_rx) = output();
        let (miso_data, miso_data_rx) = output();
        let (transactions_tx, transactions_rx) = bounded::<ChannelMessage<ProtocolPacket>>(16);
        let outputs = [
            OutputPort::new_with_watchdog(mosi_words, &wd, "spi", "mosi_words"),
            OutputPort::new_with_watchdog(mosi_bits, &wd, "spi", "mosi_bits"),
            OutputPort::new_with_watchdog(mosi_data, &wd, "spi", "mosi_data"),
            OutputPort::new_with_watchdog(miso_words, &wd, "spi", "miso_words"),
            OutputPort::new_with_watchdog(miso_bits, &wd, "spi", "miso_bits"),
            OutputPort::new_with_watchdog(miso_data, &wd, "spi", "miso_data"),
            OutputPort::new_with_watchdog(
                Sender::new(vec![transactions_tx]),
                &wd,
                "spi",
                "transactions",
            ),
        ];

        let mut decoder = SpiDecoder::new(SpiMode::Mode0, 4, true, true);
        loop {
            match decoder.work(&inputs, &outputs) {
                Ok(_) => {}
                Err(WorkError::Shutdown) => break,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }

        let collect = |rx: crossbeam_channel::Receiver<ChannelMessage<Word>>| -> Vec<Word> {
            rx.try_iter()
                .flat_map(|message| match message {
                    ChannelMessage::Sample(word) => vec![word],
                    ChannelMessage::Batch(words) => words,
                    ChannelMessage::EndOfStream => vec![],
                })
                .collect()
        };
        // The word spans its sampling edges: first at 100ns, last at 700ns.
        assert_eq!(
            collect(mosi_words_rx),
            vec![Word::spanning(0b1010, 100, 600)]
        );
        assert_eq!(
            collect(miso_words_rx),
            vec![Word::spanning(0b0101, 100, 600)]
        );
        assert_eq!(
            collect(mosi_bits_rx),
            vec![
                Word::spanning(1, 0, 200),
                Word::spanning(0, 200, 200),
                Word::spanning(1, 400, 200),
                Word::spanning(0, 600, 200),
            ]
        );
        assert_eq!(
            collect(miso_bits_rx),
            vec![
                Word::spanning(0, 0, 200),
                Word::spanning(1, 200, 200),
                Word::spanning(0, 400, 200),
                Word::spanning(1, 600, 200),
            ]
        );
        assert_eq!(collect(mosi_data_rx), vec![Word::spanning(0b1010, 0, 800)]);
        assert_eq!(collect(miso_data_rx), vec![Word::spanning(0b0101, 0, 800)]);
        let transaction = transactions_rx
            .try_iter()
            .find_map(|message| match message {
                ChannelMessage::Sample(packet) => Some(packet),
                ChannelMessage::Batch(mut packets) => packets.pop(),
                ChannelMessage::EndOfStream => None,
            })
            .expect("the CS window should emit one transaction");
        assert_eq!(transaction.protocol_id, SPI_TRANSACTION_PROTOCOL_ID);
        assert_eq!(transaction.start_time_ns, 0);
        assert_eq!(transaction.end_time_ns, 1_000);
        let ProtocolValue::Mapping(fields) = transaction.value else {
            panic!("transaction value should be a mapping");
        };
        assert_eq!(fields["complete"], ProtocolValue::Bool(true));
        assert_eq!(
            fields["mosi"],
            ProtocolValue::List(vec![ProtocolValue::Integer(0b1010)])
        );
        assert_eq!(
            fields["miso"],
            ProtocolValue::List(vec![ProtocolValue::Integer(0b0101)])
        );
    }

    #[test]
    fn work_streamed_flushes_final_active_cs_window_at_capture_end() {
        use crossbeam_channel::bounded;
        use signal_processing::{ChannelMessage, Sender, Watchdog};

        let watchdog = Watchdog::new();
        let make_input = |samples: &[Sample], port: &str| {
            let (tx, rx) = bounded::<ChannelMessage<Sample>>(samples.len() + 1);
            for &sample in samples {
                tx.send(ChannelMessage::Sample(sample)).unwrap();
            }
            drop(tx);
            InputPort::new_with_watchdog(rx, &watchdog, "spi", port)
        };

        // The second CS sample is the capture source's terminal same-level
        // edge, not an inactive transition. The complete word before that
        // boundary must still be emitted.
        let inputs = [
            make_input(&[Sample::new(false, 0), Sample::new(false, 1_000)], "cs"),
            make_input(
                &[
                    Sample::new(false, 0),
                    Sample::new(true, 100),
                    Sample::new(false, 200),
                    Sample::new(true, 300),
                    Sample::new(false, 400),
                    Sample::new(true, 500),
                    Sample::new(false, 600),
                    Sample::new(true, 700),
                    Sample::new(true, 1_000),
                ],
                "clk",
            ),
            make_input(
                &[
                    Sample::new(true, 0),
                    Sample::new(false, 200),
                    Sample::new(true, 400),
                    Sample::new(false, 600),
                    Sample::new(false, 1_000),
                ],
                "mosi",
            ),
        ];
        let (output_tx, output_rx) = bounded::<ChannelMessage<Word>>(4);
        let outputs = [OutputPort::new_with_watchdog(
            Sender::new(vec![output_tx]),
            &watchdog,
            "spi",
            "mosi_words",
        )];
        let mut decoder = SpiDecoder::new(SpiMode::Mode0, 4, true, false);

        assert_eq!(decoder.work(&inputs, &outputs).unwrap(), 1);
        assert!(matches!(
            decoder.work(&inputs, &outputs),
            Err(WorkError::Shutdown)
        ));
        let words: Vec<_> = output_rx
            .try_iter()
            .flat_map(|message| match message {
                ChannelMessage::Sample(word) => vec![word],
                ChannelMessage::Batch(words) => words,
                ChannelMessage::EndOfStream => vec![],
            })
            .collect();
        assert_eq!(words, vec![Word::spanning(0b1010, 100, 600)]);
    }
}
