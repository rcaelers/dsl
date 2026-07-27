//! Output-file creation and persistence owned by concrete file sinks.

mod implementation;
#[cfg(test)]
mod testing;

pub(crate) use implementation::{NativeOutputStorage, OutputFile, OutputStorage};
#[cfg(test)]
pub(crate) use testing::TestOutputStorage;
