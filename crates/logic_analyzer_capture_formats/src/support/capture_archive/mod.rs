//! Capture-archive access independent of a concrete container or host filesystem.

mod archive;
mod file_byte_source;

pub(crate) use archive::{CaptureArchive, ZipCaptureArchive};
pub(crate) use file_byte_source::FileByteSource;
