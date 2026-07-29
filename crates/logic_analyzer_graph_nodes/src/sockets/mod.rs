//! Socket definitions and node-category colors for built-in graph nodes.

mod category_colors;
mod number;
mod protocol_packets;
mod signal;
mod text;
mod text_path;
mod timeline_marker;
mod trigger;
mod words;

pub(crate) use category_colors::{COLOR_DECODERS, COLOR_LOGIC, COLOR_OUTPUT, COLOR_SOURCES};
pub(crate) use number::Number;
pub(crate) use protocol_packets::ProtocolPackets;
pub(crate) use signal::Signal;
pub(crate) use text::Text;
pub(crate) use text_path::TextPath;
pub(crate) use timeline_marker::TimelineMarker;
pub(crate) use trigger::Trigger;
pub(crate) use words::Words;
