use std::time::Duration;

use super::implementation::AppManager;
use crate::manager::{DisconnectEvent, InputSub, NodeFailure, NodeSpec};
use crate::node::{ConfigurationBoundary, NodeConfig, ProcessNode};

/// Execution behavior behind the portable application-runtime facade.
pub trait AppManagerBackend {
    fn is_finished(&self) -> bool;

    fn add_node(&mut self, spec: NodeSpec) -> Result<(), String>;

    fn add_node_deferred(&mut self, spec: NodeSpec) -> Result<(), String>;

    fn start_all_deferred(&mut self) -> Result<(), String>;

    fn remove_node(&mut self, name: &str) -> Result<(), String>;

    fn reconfigure(&mut self, name: &str, config: NodeConfig) -> Result<(), String>;

    fn reconfigure_at(
        &mut self,
        name: &str,
        config: NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> Result<(), String>;

    fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn ProcessNode>,
        inputs: Vec<Option<InputSub>>,
    ) -> Result<(), String>;

    fn progress(&self) -> Vec<(String, u64)>;

    fn take_disconnected(&self) -> Vec<DisconnectEvent>;

    fn take_failures(&mut self) -> Vec<NodeFailure> {
        Vec::new()
    }

    fn request_stop(&mut self);

    fn wait(&mut self);

    fn pump(&mut self, budget: usize);

    fn pump_for(&mut self, budget: usize, _max_duration: Duration) {
        self.pump(budget);
    }
}

/// Constructs one application-runtime facade for each graph run.
pub trait AppManagerFactory: Send + Sync {
    fn create(&self) -> AppManager;
}
