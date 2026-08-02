//! Shared deterministic fixtures for cross-crate integration tests.

mod buffered_fake;
mod data_plane;
mod live_acquisition;

pub use buffered_fake::{BufferedFakeConfig, BufferedFakeController, BufferedFakeProvider};
pub use data_plane::{
    DerivedStoreConformanceSnapshot, RepositoryConformanceSnapshot, capture_store_conformance,
    derived_store_conformance, repository_conformance,
};
pub use live_acquisition::{
    DeterministicFakeConfig, DeterministicFakeController, DeterministicFakeProvider,
    DeterministicTrigger, DeterministicTriggerCount, DeterministicTriggerCountMode,
    DeterministicTriggerLogic, DeterministicTriggerPredicate, DeterministicTriggerStage,
};
