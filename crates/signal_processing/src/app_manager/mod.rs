//! Platform-neutral application runtime manager facade.

mod contract;
mod cooperative;
mod implementation;

pub use contract::{AppManagerBackend, AppManagerFactory};
pub use cooperative::{CooperativeAppManagerBackend, CooperativeAppManagerFactory};
pub use implementation::AppManager;
