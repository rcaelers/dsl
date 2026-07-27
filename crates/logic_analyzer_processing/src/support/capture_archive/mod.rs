//! Capture-archive access independent of a concrete container or host filesystem.

mod implementation;

pub(crate) use implementation::{CaptureArchive, ZipCaptureArchive};
