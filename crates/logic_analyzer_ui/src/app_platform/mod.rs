//! Application-shell state and host-effect orchestration.
//!
//! **Owned data and invariants.** `PlatformState` owns document identity, the saved snapshot,
//! recent-file ordering, guarded destructive actions, capture-presentation identity, and
//! confirmation state.
//!
//! **Facade.** The state methods and `App::platform_*` hooks form the behavior boundary used by the
//! crate root.
//!
//! **Permitted owner dependencies.** The module consumes UI-owned host ports, portable
//! graph/runtime contracts, eframe persistence, and egui dialogs.
//!
//! **Explicit exclusions.** It does not implement filesystem or dialog mechanisms, select a
//! compilation target, own graph semantics, or contain native/web adapters despite its
//! application-facing name.

mod confirmation_dialog;
mod hooks;
mod state;
mod ui_persistence;

pub(crate) use state::PlatformState;
pub(crate) use ui_persistence::PersistedUiState;
