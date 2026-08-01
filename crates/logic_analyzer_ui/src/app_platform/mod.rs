//! Platform-specific application/document state.

#[cfg(not(target_arch = "wasm32"))]
#[path = "native.rs"]
mod implementation;
#[cfg(target_arch = "wasm32")]
#[path = "wasm.rs"]
mod implementation;

#[cfg(not(target_arch = "wasm32"))]
#[path = "native_hooks.rs"]
mod hooks;
#[cfg(target_arch = "wasm32")]
#[path = "wasm_hooks.rs"]
mod hooks;
mod ui_persistence;

pub(crate) use implementation::PlatformState;
#[cfg(target_os = "macos")]
pub(crate) use implementation::notify_recent_files_changed;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use implementation::{FileCommand, GuardedAction, capture_session_directory};
#[cfg(target_os = "macos")]
pub use implementation::{
    NativeMenuCommand, dispatch_native_menu_command, set_recent_files_listener,
};
pub(crate) use ui_persistence::PersistedUiState;
