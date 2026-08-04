//! Platform-neutral immutable byte regions, artifact identities, repositories, and replication.
//!
//! This crate owns byte-source and artifact persistence contracts. It does not own capture
//! formats, derived-data encodings, runtime execution, or host-specific storage adapters.

mod artifact;
mod contract;
mod memory;
mod replication;
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
pub use replication::{
    ArtifactReplicationEvent, ArtifactReplicationReceiver, ReplicatingArtifactRepository,
};
pub use repository_source::{ArtifactByteSource, read_artifact_region};
