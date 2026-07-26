use super::contract::HostService;
use super::platform_contract::PlatformHostService;

struct WebHostService;

impl PlatformHostService for WebHostService {}

impl HostService for WebHostService {}

pub(crate) fn standard_host_service() -> Box<dyn HostService> {
    Box::new(WebHostService)
}
