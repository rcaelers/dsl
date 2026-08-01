#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::standard_services;
#[cfg(target_arch = "wasm32")]
pub(crate) use web::standard_services;
