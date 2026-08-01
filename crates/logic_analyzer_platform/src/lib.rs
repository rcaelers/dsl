//! Target-selected adapters for services owned by reusable core crates.
//!
//! This crate is the composition boundary for host APIs. Core crates define
//! portable contracts and receive their implementations from application
//! roots; they never depend on this crate.

mod platform;
mod services;

#[cfg(target_os = "macos")]
pub use platform::{dispatch_host_command, set_recent_files_listener};
pub use services::PlatformServices;

/// Builds the services appropriate for the selected application host.
pub fn standard_services() -> PlatformServices {
    platform::standard_services()
}
