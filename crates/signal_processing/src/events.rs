//! Event and level-stream types for control-path channels.
//!
//! Two kinds of stream flow between control nodes (see
//! `docs/PIPELINE_DESIGN.md`):
//!
//! - **Events** ([`Trigger`], [`Word`]): timestamped occurrences with no
//!   value between occurrences. They can only be reacted to.
//! - **Stepped levels** ([`NumberSample`], [`TextSample`]): a value defined at
//!   *every* instant, transmitted as changes only — the same RLE idea as
//!   [`Sample`](super::sample::Sample). Every level producer emits its initial
//!   value at t=0 on its first `work()` call, and consumers hold the last
//!   received value, so a consumer never has to wait for a level to *exist*.
//!
//! All timestamps are nanoseconds, in the same domain as `Sample.start_time_ns`.

use std::collections::BTreeMap;
use std::sync::Arc;

/// Longest inferred display span for an instantaneous word when no recent
/// cadence is available. Prevents sparse word events from painting a value
/// continuously across an unrelated or gated-off interval.
pub const MAX_ANNOTATION_NS: u64 = 100_000_000;

/// Returns the visual end of an instantaneous word with a known successor.
///
/// Adjacent words in a burst still meet exactly. When the next word is much
/// later than the recent cadence, the current word closes after one expected
/// period so the intervening interval remains visibly empty.
///
/// # Parameters
/// - `previous_start_ns`: Prior word start used to infer current burst cadence.
/// - `start_ns`: Start of the instantaneous word being displayed.
/// - `next_start_ns`: Start of the following word.
pub fn instantaneous_word_end_ns(
    previous_start_ns: Option<u64>,
    start_ns: u64,
    next_start_ns: u64,
) -> u64 {
    instantaneous_word_end_ns_with_limit(
        previous_start_ns,
        start_ns,
        next_start_ns,
        MAX_ANNOTATION_NS,
    )
}

pub(crate) fn instantaneous_word_end_ns_with_limit(
    previous_start_ns: Option<u64>,
    start_ns: u64,
    next_start_ns: u64,
    max_span_ns: u64,
) -> u64 {
    let gap_ns = next_start_ns.saturating_sub(start_ns);
    let inferred_limit_ns = previous_start_ns
        .map(|previous| start_ns.saturating_sub(previous))
        .filter(|interval| *interval > 0)
        .unwrap_or(max_span_ns)
        .min(max_span_ns);
    start_ns.saturating_add(gap_ns.min(inferred_limit_ns))
}

/// Instantaneous event (e.g. a matcher hit). No payload beyond time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trigger {
    /// Timestamp in nanoseconds.
    pub timestamp_ns: u64,
}

/// A persisted point on the shared graph timeline.
///
/// The name and stable identity belong to the graph/host contract; processing
/// nodes only transport the timestamp needed to derive events and levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineMarker {
    /// Timestamp in nanoseconds.
    pub timestamp_ns: u64,
}

impl TimelineMarker {
    /// Creates a timeline marker at a shared nanosecond timestamp.
    pub fn new(timestamp_ns: u64) -> Self {
        Self { timestamp_ns }
    }
}

/// Open, protocol-neutral value transported between independently authored
/// decoder nodes. Concrete protocol contracts determine which shapes are
/// meaningful on a connection.
#[derive(Clone, Debug, PartialEq)]
pub enum ProtocolValue {
    /// No structured value.
    Null,
    /// Boolean structured value.
    Bool(bool),
    /// Signed integer structured value.
    Integer(i128),
    /// Floating-point structured value.
    Float(f64),
    /// UTF-8 string structured value.
    String(String),
    /// Arbitrary immutable bytes.
    Bytes(Arc<[u8]>),
    /// Ordered collection of structured values.
    List(Vec<Self>),
    /// Fixed-position structured values.
    Tuple(Vec<Self>),
    /// Named structured values.
    Mapping(BTreeMap<String, Self>),
}

/// Timestamped structured value exchanged by stacked protocol decoders.
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolPacket {
    /// Source-domain sample coordinates. Producers derived from payloads that
    /// carry time but no sample position set both sample coordinates to zero.
    pub start_sample: u64,
    pub end_sample: u64,
    /// Shared timeline start timestamp.
    pub start_time_ns: u64,
    /// Shared timeline end timestamp.
    pub end_time_ns: u64,
    /// Stable protocol identity owned by the decoder.
    pub protocol_id: String,
    /// Protocol-owned structured packet value.
    pub value: ProtocolValue,
}

impl ProtocolPacket {
    /// Returns a bounded protocol-neutral fallback display string.
    pub fn display_text(&self) -> String {
        let value = match &self.value {
            ProtocolValue::Null => "null".into(),
            ProtocolValue::Bool(value) => value.to_string(),
            ProtocolValue::Integer(value) => value.to_string(),
            ProtocolValue::Float(value) => value.to_string(),
            ProtocolValue::String(value) => value.clone(),
            ProtocolValue::Bytes(value) => format!("{} bytes", value.len()),
            ProtocolValue::List(value) => format!("list[{}]", value.len()),
            ProtocolValue::Tuple(value) => format!("tuple[{}]", value.len()),
            ProtocolValue::Mapping(value) => format!("map[{}]", value.len()),
        };
        format!("{} · {value}", self.protocol_id)
    }
}

impl Trigger {
    /// Creates an instantaneous trigger event at a shared nanosecond timestamp.
    ///
    /// # Parameters
    /// - `timestamp_ns`: Shared timeline timestamp of the event.
    pub fn new(timestamp_ns: u64) -> Self {
        Self { timestamp_ns }
    }
}

/// Optional non-numeric content carried by a decoded word.
///
/// Numeric words remain allocation-free. Byte words retain their complete,
/// arbitrary-width value, while text gives decoders a generic way to attach a
/// preferred label without introducing protocol-specific payload types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WordPayload {
    /// Arbitrary-width immutable decoded bytes.
    Bytes(Arc<[u8]>),
    /// Decoder-provided text label.
    Text(Arc<str>),
}

/// A single decoded item from any framed or sampled word stream.
///
/// `value` is the numeric representation used by native decoders. `payload`
/// carries arbitrary-width bytes or a decoder-provided label when the item is
/// not adequately represented by a `u64` alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    /// Numeric decoder value or presentation tag.
    pub value: u64,
    /// Optional arbitrary-width or textual decoder payload.
    pub payload: Option<WordPayload>,
    /// Timestamp of the word's start (its first sampling edge), ns.
    pub timestamp_ns: u64,
    /// The word's real extent: start to its last sampling edge / frame
    /// end, ns. `0` means instantaneous. The viewer joins adjacent
    /// instantaneous words within a decode burst, but leaves long gaps
    /// empty rather than implying valid decoded data while a gate is off.
    pub duration_ns: u64,
}

impl Word {
    /// An instantaneous word (`duration_ns == 0`).
    pub fn new(value: u64, timestamp_ns: u64) -> Self {
        Self {
            value,
            payload: None,
            timestamp_ns,
            duration_ns: 0,
        }
    }

    /// A word spanning `[timestamp_ns, timestamp_ns + duration_ns]`.
    pub fn spanning(value: u64, timestamp_ns: u64, duration_ns: u64) -> Self {
        Self {
            value,
            payload: None,
            timestamp_ns,
            duration_ns,
        }
    }

    /// An arbitrary-width byte word spanning the supplied interval.
    pub fn bytes(value: impl Into<Arc<[u8]>>, timestamp_ns: u64, duration_ns: u64) -> Self {
        Self::bytes_with_tag(0, value, timestamp_ns, duration_ns)
    }

    /// An arbitrary-width byte word with a numeric presentation tag.
    pub fn bytes_with_tag(
        tag: u64,
        value: impl Into<Arc<[u8]>>,
        timestamp_ns: u64,
        duration_ns: u64,
    ) -> Self {
        Self {
            value: tag,
            payload: Some(WordPayload::Bytes(value.into())),
            timestamp_ns,
            duration_ns,
        }
    }

    /// A labeled decoded item spanning the supplied interval.
    pub fn text(value: impl Into<Arc<str>>, timestamp_ns: u64, duration_ns: u64) -> Self {
        Self {
            value: 0,
            payload: Some(WordPayload::Text(value.into())),
            timestamp_ns,
            duration_ns,
        }
    }

    /// A numeric word with an explicit presentation label.
    ///
    /// # Parameters
    /// - `value`: Numeric decoder value.
    /// - `label`: Decoder-provided presentation text.
    /// - `timestamp_ns`: Shared timeline start timestamp.
    /// - `duration_ns`: Explicit duration in nanoseconds, or zero for an event.
    pub fn labeled(
        value: u64,
        label: impl Into<Arc<str>>,
        timestamp_ns: u64,
        duration_ns: u64,
    ) -> Self {
        Self {
            value,
            payload: Some(WordPayload::Text(label.into())),
            timestamp_ns,
            duration_ns,
        }
    }

    /// Returns whether numeric.
    pub fn is_numeric(&self) -> bool {
        self.payload.is_none()
    }

    /// The word's end (equals its start for instantaneous words).
    pub fn end_ns(&self) -> u64 {
        self.timestamp_ns + self.duration_ns
    }
}

/// A decoded word prepared for timeline rendering. Instantaneous words use
/// the next word's timestamp as `end_ns`; explicitly-spanning words retain
/// their encoded duration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    /// Inclusive timeline start of the rendered annotation.
    pub start_ns: u64,
    /// Exclusive or inferred timeline end of the rendered annotation.
    pub end_ns: u64,
    /// Numeric decoder value or presentation tag.
    pub value: u64,
    /// Optional arbitrary-width or textual decoder payload.
    pub payload: Option<WordPayload>,
}

/// Change of an integer level (e.g. counter output). Mirrors `Sample`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberSample {
    /// The level's value from `start_time_ns` until the next change.
    pub value: i64,
    /// Timestamp in nanoseconds when this value started.
    pub start_time_ns: u64,
}

impl NumberSample {
    /// Creates an integer level beginning at a shared timeline timestamp.
    pub fn new(value: i64, start_time_ns: u64) -> Self {
        Self {
            value,
            start_time_ns,
        }
    }
}

/// Change of a text level (e.g. formatter output / filename). Mirrors `Sample`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSample {
    /// The level's value from `start_time_ns` until the next change.
    pub value: String,
    /// Timestamp in nanoseconds when this value started.
    pub start_time_ns: u64,
}

impl TextSample {
    /// Creates a text level beginning at a shared timeline timestamp.
    pub fn new(value: impl Into<String>, start_time_ns: u64) -> Self {
        Self {
            value: value.into(),
            start_time_ns,
        }
    }
}
