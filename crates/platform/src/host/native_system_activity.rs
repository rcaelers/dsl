//! Native scoped system-activity adapter.

use std::sync::Arc;

use platform_runtime::{
    ObservedSystemActivityLease, SystemActivityInterruption, SystemActivityLease,
    SystemActivityManager,
};

/// Creates a native system-activity manager for one application.
pub fn system_activity_manager(
    application_name: &str,
    application_id: &str,
) -> Arc<dyn SystemActivityManager> {
    Arc::new(NativeSystemActivityManager {
        application_name: application_name.to_owned(),
        application_id: application_id.to_owned(),
    })
}

struct NativeSystemActivityManager {
    application_name: String,
    application_id: String,
}

impl SystemActivityManager for NativeSystemActivityManager {
    fn begin_activity(&self, reason: &str) -> Box<dyn SystemActivityLease> {
        let inhibitor = keepawake::Builder::default()
            .idle(true)
            .reason(reason)
            .app_name(&self.application_name)
            .app_reverse_domain(&self.application_id)
            .create()
            .map_err(|error| {
                tracing::warn!(%error, "system sleep inhibition is unavailable");
                error
            })
            .ok();
        Box::new(NativeSystemActivityLease {
            observation: ObservedSystemActivityLease::new(inhibitor.is_some()),
            _inhibitor: inhibitor,
        })
    }
}

struct NativeSystemActivityLease {
    observation: ObservedSystemActivityLease,
    _inhibitor: Option<keepawake::KeepAwake>,
}

impl SystemActivityLease for NativeSystemActivityLease {
    fn sleep_inhibited(&self) -> bool {
        self.observation.sleep_inhibited()
    }

    fn poll_interruption(&mut self) -> Option<SystemActivityInterruption> {
        self.observation.poll_interruption()
    }
}
