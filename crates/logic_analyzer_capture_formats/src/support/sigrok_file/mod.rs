//! Random-access sigrok v2 (`.sr`) capture-file support.

mod reader;

pub(crate) use reader::{SigrokCapture, SigrokFileCaptureDataSource};
