//! Random-access support for DSLogic `.dsl` capture files.

mod reader;

#[cfg(test)]
pub(crate) use reader::DslCaptureReader;
pub(crate) use reader::{DslChunkedCaptureReader, DslFileCaptureDataSource, parse_header};
