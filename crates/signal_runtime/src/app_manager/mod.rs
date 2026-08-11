//! Platform-neutral application runtime manager facade.

mod contract;
mod cooperative;
mod manager;
mod pipeline;

pub use contract::{AppManagerBackend, AppManagerFactory};
pub use cooperative::{CooperativeAppManagerBackend, CooperativeAppManagerFactory};
pub use manager::AppManager;
pub use pipeline::PipelineAppManagerFactory;
