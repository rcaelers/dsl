use logic_analyzer_ui::{AppServices, HostService};

/// Opaque host services assembled for one application instance.
///
/// The bundle grows as storage, execution, source-acquisition, and export
/// ports become injectable. Its fields remain private so applications compose
/// through supported owner contracts rather than concrete platform types.
pub struct PlatformServices {
    ui_services: AppServices,
}

impl PlatformServices {
    pub(crate) fn with_host_service(host_service: Box<dyn HostService>) -> Self {
        Self {
            ui_services: AppServices::with_host_service(host_service),
        }
    }

    /// Returns the UI-owned services for application construction.
    pub fn into_ui_services(self) -> AppServices {
        self.ui_services
    }
}
