//! Capture-archive access independent of a concrete container or host filesystem.

mod file_byte_source;
mod implementation;

pub(crate) use file_byte_source::FileByteSource;
pub(crate) use implementation::{CaptureArchive, ZipCaptureArchive};
