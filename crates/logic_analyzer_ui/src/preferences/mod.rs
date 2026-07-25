#[cfg(not(target_arch = "wasm32"))]
#[path = "native.rs"]
mod implementation;
#[cfg(target_arch = "wasm32")]
#[path = "wasm.rs"]
mod implementation;

pub(crate) use implementation::PreferencesWindow;
