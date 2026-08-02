//! Platform-neutral byte-source and immutable-region contracts.

mod artifact;
mod contract;
mod memory;
mod repository_source;

#[cfg(test)]
mod artifact_contract_tests;
#[cfg(test)]
mod storage_contract_tests;

pub use artifact::{
    ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactRepository, MemoryArtifactRepository,
    ReadArtifact, RepositoryCapabilities, RepositoryError, WriteArtifact,
};
pub use contract::{
    ByteRange, ByteRegion, ImmutableByteRegion, PreparedByteSource, RandomAccessReader,
    SourceCapabilities, SourceIdentity, SourceReadError,
};
pub use memory::{ChunkedByteSource, OwnedByteSource};
pub use repository_source::{ArtifactByteSource, read_artifact_region};
