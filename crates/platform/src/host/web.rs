use std::sync::Arc;

use platform_artifacts::ArtifactRepository;
use platform_runtime::{ObservedSystemActivityManager, SystemActivityManager};

use super::web_artifact_repository::BrowserArtifactRepository;
use crate::ArtifactRepositoryOpenError;

/// Creates the browser system-activity fallback.
///
/// Browsers do not expose a system-sleep inhibitor. The returned manager
/// observes suspend/resume gaps so long-running host work can reject affected
/// results rather than silently accepting them.
pub fn system_activity_manager(
    _application_name: &str,
    _application_id: &str,
) -> Arc<dyn SystemActivityManager> {
    Arc::new(ObservedSystemActivityManager)
}

/// Opens a browser durable artifact repository under the supplied OPFS root.
pub async fn open_browser_artifact_repository(
    root_name: &str,
) -> Result<Arc<dyn ArtifactRepository>, ArtifactRepositoryOpenError> {
    BrowserArtifactRepository::open(root_name)
        .await
        .map(|repository| Arc::new(repository) as Arc<dyn ArtifactRepository>)
}

/// Chooses a conservative browser worker count from host concurrency.
pub fn browser_worker_parallelism() -> usize {
    web_sys::window()
        .map(|window| window.navigator().hardware_concurrency() as usize)
        .unwrap_or(1)
        .saturating_sub(1)
        .clamp(1, 8)
}
