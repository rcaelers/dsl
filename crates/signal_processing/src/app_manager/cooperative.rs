use super::contract::{AppManagerBackend, AppManagerFactory};
use super::implementation::AppManager;
use crate::cooperative_manager::CooperativeManager;
use crate::manager::{DisconnectEvent, InputSub, NodeSpec};
use crate::node::{ConfigurationBoundary, NodeConfig, ProcessNode};

/// Portable cooperative execution backend.
pub struct CooperativeAppManagerBackend {
    manager: CooperativeManager,
}

impl CooperativeAppManagerBackend {
    pub fn new() -> Self {
        Self {
            manager: CooperativeManager::new(),
        }
    }
}

impl Default for CooperativeAppManagerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AppManagerBackend for CooperativeAppManagerBackend {
    fn is_finished(&self) -> bool {
        self.manager.is_finished()
    }

    fn add_node(&mut self, spec: NodeSpec) -> Result<(), String> {
        self.manager.add_node(spec)
    }

    fn add_node_deferred(&mut self, spec: NodeSpec) -> Result<(), String> {
        self.manager.add_node_deferred(spec)
    }

    fn start_all_deferred(&mut self) -> Result<(), String> {
        self.manager.start_all_deferred()
    }

    fn remove_node(&mut self, name: &str) -> Result<(), String> {
        self.manager.remove_node(name)
    }

    fn reconfigure(&mut self, name: &str, config: NodeConfig) -> Result<(), String> {
        self.manager.reconfigure(name, config)
    }

    fn reconfigure_at(
        &mut self,
        name: &str,
        config: NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> Result<(), String> {
        self.manager.reconfigure_at(name, config, boundary)
    }

    fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn ProcessNode>,
        inputs: Vec<Option<InputSub>>,
    ) -> Result<(), String> {
        self.manager.restart_node(name, node, inputs)
    }

    fn progress(&self) -> Vec<(String, u64)> {
        self.manager.progress()
    }

    fn take_disconnected(&self) -> Vec<DisconnectEvent> {
        self.manager.take_disconnected()
    }

    fn request_stop(&mut self) {
        self.manager.request_stop();
    }

    fn wait(&mut self) {
        self.manager.wait();
    }

    fn pump(&mut self, budget: usize) {
        self.manager.pump(budget);
    }
}

/// Factory for portable cooperative execution.
pub struct CooperativeAppManagerFactory;

impl AppManagerFactory for CooperativeAppManagerFactory {
    fn create(&self) -> AppManager {
        AppManager::with_backend(Box::new(CooperativeAppManagerBackend::new()))
    }
}
