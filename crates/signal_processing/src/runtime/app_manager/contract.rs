use std::time::Duration;

use super::super::manager::{DisconnectEvent, InputSub, NodeFailure, NodeSpec};
use super::super::node::{ConfigurationBoundary, NodeConfig, ProcessNode};
use super::implementation::AppManager;

/// Execution behavior behind the portable application-runtime facade.
pub trait AppManagerBackend {
    /// Reports whether all nodes have reached a terminal state.
    fn is_finished(&self) -> bool;

    /// Adds and starts a node using backend-specific execution.
    ///
    /// # Parameters
    /// - `spec`: Node implementation and input wiring to own.
    fn add_node(&mut self, spec: NodeSpec) -> Result<(), String>;

    /// Registers a node without starting its execution yet.
    ///
    /// # Parameters
    ///
    /// - `spec`: Node implementation and input wiring to own.
    fn add_node_deferred(&mut self, spec: NodeSpec) -> Result<(), String>;

    /// Starts all nodes previously added through [`Self::add_node_deferred`].
    fn start_all_deferred(&mut self) -> Result<(), String>;

    /// Stops, detaches, and removes one node.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local name of the node to remove.
    fn remove_node(&mut self, name: &str) -> Result<(), String>;

    /// Applies a validated hot configuration to one node.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local name of the node.
    /// - `config`: Configuration to apply.
    fn reconfigure(&mut self, name: &str, config: NodeConfig) -> Result<(), String>;

    /// Schedules hot configuration at an event-time boundary.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local name of the node.
    /// - `config`: Configuration to schedule.
    /// - `boundary`: Event-time boundary that activates the configuration.
    fn reconfigure_at(
        &mut self,
        name: &str,
        config: NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> Result<(), String>;

    /// Replaces a node while retaining its downstream subscriptions.
    ///
    /// # Parameters
    ///
    /// - `name`: Graph-local name of the node to replace.
    /// - `node`: Fresh node implementation.
    /// - `inputs`: Replacement input wiring in schema order.
    fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn ProcessNode>,
        inputs: Vec<Option<InputSub>>,
    ) -> Result<(), String>;

    /// Returns cumulative produced-item counts by node name.
    fn progress(&self) -> Vec<(String, u64)>;

    /// Returns disconnect-policy events accumulated since the previous call.
    fn take_disconnected(&self) -> Vec<DisconnectEvent>;

    /// Returns and clears terminal node failures.
    fn take_failures(&mut self) -> Vec<NodeFailure> {
        Vec::new()
    }

    /// Requests non-blocking cancellation of all managed work.
    fn request_stop(&mut self);

    /// Waits for backend-owned work to finish and reaps it.
    fn wait(&mut self);

    /// Advances cooperative work without exceeding the supplied call budget.
    ///
    /// # Parameters
    ///
    /// - `budget`: Maximum work calls to make.
    fn pump(&mut self, budget: usize);

    /// Advances cooperative work subject to call and time budgets.
    ///
    /// # Parameters
    ///
    /// - `budget`: Maximum work calls to make.
    /// - `_max_duration`: Host-time budget; default backends may ignore it.
    fn pump_for(&mut self, budget: usize, _max_duration: Duration) {
        self.pump(budget);
    }
}

/// Constructs one application-runtime facade for each graph run.
pub trait AppManagerFactory: Send + Sync {
    /// Creates a new independent application-runtime facade.
    fn create(&self) -> AppManager;
}
