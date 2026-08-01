//! Output-file creation and persistence owned by concrete file sinks.

mod implementation;
#[cfg(test)]
mod testing;

pub(crate) use implementation::UnavailableOutputStorage;
pub use implementation::{OutputFile, OutputStorage};
#[cfg(test)]
pub(crate) use testing::TestOutputStorage;
