use std::collections::BTreeMap;
use std::sync::Arc;

/// Open structured value transported between independently authored protocol decoders.
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
