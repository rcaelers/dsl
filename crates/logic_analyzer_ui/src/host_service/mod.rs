//! UI-owned boundary for dialogs, document persistence, and cache commands.

#[cfg(test)]
mod architecture_tests;
mod contract;
#[cfg(test)]
mod host_service_tests;

pub use contract::{
    CacheClearStats, CacheEntrySnapshot, DecodedBlockCacheSnapshot, HostCommand, HostService,
    OpenDialog, SaveDialog,
};
