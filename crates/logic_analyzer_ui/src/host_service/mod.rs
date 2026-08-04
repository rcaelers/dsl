//! UI-owned boundary for dialogs, document persistence, and host commands.

#[cfg(test)]
mod architecture_tests;
mod contract;
#[cfg(test)]
mod host_service_tests;

pub use contract::{
    DecodedBlockCacheSnapshot, DownloadableOutput, HostCommand, HostService, HostUiCapabilities,
    ModifierKeyLabels, OpenDialog, SaveDialog,
};
