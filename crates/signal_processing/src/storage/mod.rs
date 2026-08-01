//! Platform-neutral byte-source and immutable-region contracts.

mod contract;
mod memory;

#[cfg(test)]
mod storage_contract_tests;

pub use contract::{
    ByteRange, ImmutableByteRegion, PreparedByteSource, RandomAccessReader, SourceCapabilities,
    SourceIdentity, SourceReadError,
};
pub use memory::OwnedByteSource;
