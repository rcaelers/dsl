use std::time::Duration;

use super::contract::AppManagerBackend;
use super::cooperative::CooperativeAppManagerBackend;
use crate::manager::{DisconnectEvent, InputSub, NodeFailure, NodeSpec};
use crate::node::{ConfigurationBoundary, NodeConfig, ProcessNode};

/// Platform-neutral facade for owning one graph run.
///
/// Applications use this stable interface while their composition root injects
/// the threaded or cooperative execution backend appropriate for the host.
pub struct AppManager {
    backend: Box<dyn AppManagerBackend>,
}

impl AppManager {
    /// Constructs the portable cooperative backend when none is injected.
    pub fn new() -> Self {
        Self::with_backend(Box::new(CooperativeAppManagerBackend::new()))
    }

    /// Constructs a manager with host-selected execution behavior.
    ///
    /// # Parameters
    /// - `backend`: Execution implementation selected by the composition root.
    pub fn with_backend(backend: Box<dyn AppManagerBackend>) -> Self {
        Self { backend }
    }

    /// Returns whether every managed node has reached a terminal state.
    pub fn is_finished(&self) -> bool {
        self.backend.is_finished()
    }

    /// Adds and starts a node.
    ///
    /// # Parameters
    ///
    /// - `spec`: Node implementation and input wiring to own.
    pub fn add_node(&mut self, spec: NodeSpec) -> Result<(), String> {
        self.backend.add_node(spec)
    }

    /// Registers a node without starting it.
    ///
    /// # Parameters
    ///
    /// - `spec`: Node implementation and input wiring to own.
    pub fn add_node_deferred(&mut self, spec: NodeSpec) -> Result<(), String> {
        self.backend.add_node_deferred(spec)
    }

    /// Starts all nodes registered through [`Self::add_node_deferred`].
    pub fn start_all_deferred(&mut self) -> Result<(), String> {
        self.backend.start_all_deferred()
    }

    /// Stops, detaches, and removes one node.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local name of the node to remove.
    pub fn remove_node(&mut self, name: &str) -> Result<(), String> {
        self.backend.remove_node(name)
    }

    /// Applies a validated hot configuration.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local name of the node.
    /// - `config`: Configuration to apply.
    pub fn reconfigure(&mut self, name: &str, config: NodeConfig) -> Result<(), String> {
        self.backend.reconfigure(name, config)
    }

    /// Schedules hot configuration at an event-time boundary.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local name of the node.
    /// - `config`: Configuration to schedule.
    /// - `boundary`: Event-time boundary that activates it.
    pub fn reconfigure_at(
        &mut self,
        name: &str,
        config: NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> Result<(), String> {
        self.backend.reconfigure_at(name, config, boundary)
    }

    /// Replaces a node while preserving its downstream subscriptions.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local name of the node to replace.
    /// - `node`: Fresh node implementation.
    /// - `inputs`: Replacement input wiring in schema order.
    pub fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn ProcessNode>,
        inputs: Vec<Option<InputSub>>,
    ) -> Result<(), String> {
        self.backend.restart_node(name, node, inputs)
    }

    /// Returns cumulative produced-item counts by node name.
    pub fn progress(&self) -> Vec<(String, u64)> {
        self.backend.progress()
    }

    /// Returns disconnect-policy events accumulated since the previous call.
    pub fn take_disconnected(&self) -> Vec<DisconnectEvent> {
        self.backend.take_disconnected()
    }

    /// Returns and clears terminal node failures.
    pub fn take_failures(&mut self) -> Vec<NodeFailure> {
        self.backend.take_failures()
    }

    /// Requests non-blocking cancellation of all managed work.
    pub fn request_stop(&mut self) {
        self.backend.request_stop();
    }

    /// Waits for managed work to finish and reaps it.
    pub fn wait(&mut self) {
        self.backend.wait();
    }

    /// Advances cooperative work within a call budget.
    ///
    /// # Parameters
    /// - `budget`: Maximum work calls to make.
    pub fn pump(&mut self, budget: usize) {
        self.backend.pump(budget);
    }

    /// Advances cooperative work within call and time budgets.
    ///
    /// # Parameters
    ///
    /// - `budget`: Maximum work calls to make.
    /// - `max_duration`: Maximum host time to spend in this call.
    pub fn pump_for(&mut self, budget: usize, max_duration: Duration) {
        self.backend.pump_for(budget, max_duration);
    }
}

impl Default for AppManager {
    fn default() -> Self {
        Self::new()
    }
}
