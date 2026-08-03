use std::time::Duration;

use super::contract::AppManagerBackend;
use super::cooperative::CooperativeAppManagerBackend;
use crate::manager::{DisconnectEvent, InputSub, NodeSpec};
use crate::node::{ConfigurationBoundary, NodeConfig, ProcessNode};

/// Platform-neutral application runtime manager.
pub struct AppManager {
    backend: Box<dyn AppManagerBackend>,
}

impl AppManager {
    /// Constructs a portable cooperative manager when no host backend was injected.
    pub fn new() -> Self {
        Self::with_backend(Box::new(CooperativeAppManagerBackend::new()))
    }

    /// Constructs a manager with host-selected execution behavior.
    pub fn with_backend(backend: Box<dyn AppManagerBackend>) -> Self {
        Self { backend }
    }

    pub fn is_finished(&self) -> bool {
        self.backend.is_finished()
    }

    pub fn add_node(&mut self, spec: NodeSpec) -> Result<(), String> {
        self.backend.add_node(spec)
    }

    pub fn add_node_deferred(&mut self, spec: NodeSpec) -> Result<(), String> {
        self.backend.add_node_deferred(spec)
    }

    pub fn start_all_deferred(&mut self) -> Result<(), String> {
        self.backend.start_all_deferred()
    }

    pub fn remove_node(&mut self, name: &str) -> Result<(), String> {
        self.backend.remove_node(name)
    }

    pub fn reconfigure(&mut self, name: &str, config: NodeConfig) -> Result<(), String> {
        self.backend.reconfigure(name, config)
    }

    pub fn reconfigure_at(
        &mut self,
        name: &str,
        config: NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> Result<(), String> {
        self.backend.reconfigure_at(name, config, boundary)
    }

    pub fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn ProcessNode>,
        inputs: Vec<Option<InputSub>>,
    ) -> Result<(), String> {
        self.backend.restart_node(name, node, inputs)
    }

    pub fn progress(&self) -> Vec<(String, u64)> {
        self.backend.progress()
    }

    pub fn take_disconnected(&self) -> Vec<DisconnectEvent> {
        self.backend.take_disconnected()
    }

    pub fn request_stop(&mut self) {
        self.backend.request_stop();
    }

    pub fn wait(&mut self) {
        self.backend.wait();
    }

    pub fn pump(&mut self, budget: usize) {
        self.backend.pump(budget);
    }

    pub fn pump_for(&mut self, budget: usize, max_duration: Duration) {
        self.backend.pump_for(budget, max_duration);
    }
}

impl Default for AppManager {
    fn default() -> Self {
        Self::new()
    }
}
