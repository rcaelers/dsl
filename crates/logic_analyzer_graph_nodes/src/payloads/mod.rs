//! Built-in retained-payload capabilities and presentations.

mod digital;
mod number;
#[cfg(test)]
mod presentation_tests;
mod protocol_packet;
mod text;
mod trigger;
mod word;

pub(crate) use word::WordSnapshotRenderer;
