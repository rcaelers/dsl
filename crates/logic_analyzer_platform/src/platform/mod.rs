#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_os = "macos")]
pub use native::set_recent_files_listener;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::standard_services;
#[cfg(target_arch = "wasm32")]
pub(crate) use web::standard_services;
