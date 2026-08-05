//! Platform-neutral immutable byte regions, artifact identities, repositories, integrity, and replication.
//!
//! This crate owns byte-source and artifact persistence contracts plus portable metadata time and
//! integrity primitives shared by persisted domains. It does not own capture
//! formats, derived-data encodings, runtime execution, or host-specific storage adapters.

mod artifact;
mod contract;
mod crc32c;
mod memory;
mod replication;
mod repository_source;
mod time_source;

#[cfg(test)]
mod artifact_contract_tests;
#[cfg(test)]
mod storage_contract_tests;
#[cfg(test)]
mod wasm_store_tests;

pub use artifact::{
    ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactRepository, MemoryArtifactRepository,
    ReadArtifact, RepositoryCapabilities, RepositoryError, WriteArtifact,
};
pub use contract::{
    ByteRange, ByteRegion, ImmutableByteRegion, PreparedByteSource, RandomAccessReader,
    SourceCapabilities, SourceIdentity, SourceReadError,
};
pub use crc32c::{block_checksum, checksum_parts};
pub use memory::{ChunkedByteSource, OwnedByteSource};
pub use replication::{
    ArtifactReplicationEvent, ArtifactReplicationReceiver, ReplicatingArtifactRepository,
};
pub use repository_source::{ArtifactByteSource, read_artifact_region};
pub use time_source::{SystemUnixTimeSource, UnixTimeSource};
