//! Socket definitions and node-category colors for built-in graph nodes.

mod category_colors;
mod number;
mod protocol_packets;
mod signal;
mod text;
mod text_open_path;
mod text_save_path;
mod trigger;
mod words;

pub(crate) use category_colors::{COLOR_DECODERS, COLOR_LOGIC, COLOR_OUTPUT, COLOR_SOURCES};
pub(crate) use number::Number;
pub(crate) use protocol_packets::ProtocolPackets;
pub(crate) use signal::Signal;
pub(crate) use text::Text;
pub(crate) use text_open_path::TextOpenPath;
pub(crate) use text_save_path::TextSavePath;
pub(crate) use trigger::Trigger;
pub(crate) use words::Words;
