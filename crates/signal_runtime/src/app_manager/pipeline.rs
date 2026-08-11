use std::sync::Arc;

use platform_runtime::WorkExecutor;

use super::contract::{AppManagerBackend, AppManagerFactory};
use super::manager::AppManager;
use crate::errors::PipelineError;
use crate::manager::{DisconnectEvent, InputSub, NodeFailure, NodeSpec, PipelineManager};
use crate::node::{ConfigurationBoundary, NodeConfig, ProcessNode};

/// Application-manager factory backed by the supervised stream pipeline.
///
/// The composition root supplies the host executor. This runtime owner keeps
/// graph supervision and node lifecycle policy independent of the native or
/// browser adapter that schedules the work.
pub struct PipelineAppManagerFactory {
    work_executor: Arc<dyn WorkExecutor>,
}

impl PipelineAppManagerFactory {
    /// Creates a factory that schedules graph work through `work_executor`.
    pub fn new(work_executor: Arc<dyn WorkExecutor>) -> Self {
        Self { work_executor }
    }
}

impl AppManagerFactory for PipelineAppManagerFactory {
    fn create(&self) -> Result<AppManager, PipelineError> {
        Ok(AppManager::with_backend(Box::new(
            PipelineAppManagerBackend {
                manager: PipelineManager::new(Arc::clone(&self.work_executor))?,
            },
        )))
    }
}

struct PipelineAppManagerBackend {
    manager: PipelineManager,
}

impl AppManagerBackend for PipelineAppManagerBackend {
    fn is_finished(&self) -> bool {
        self.manager.is_finished()
    }

    fn add_node(&mut self, spec: NodeSpec) -> Result<(), PipelineError> {
        self.manager.add_node(spec)
    }

    fn add_node_deferred(&mut self, spec: NodeSpec) -> Result<(), PipelineError> {
        self.manager.add_node_deferred(spec)
    }

    fn start_all_deferred(&mut self) -> Result<(), PipelineError> {
        self.manager.start_all_deferred()
    }

    fn remove_node(&mut self, name: &str) -> Result<(), PipelineError> {
        self.manager.remove_node(name)
    }

    fn reconfigure(&mut self, name: &str, config: NodeConfig) -> Result<(), PipelineError> {
        self.manager.reconfigure(name, config)
    }

    fn reconfigure_at(
        &mut self,
        name: &str,
        config: NodeConfig,
        boundary: ConfigurationBoundary,
    ) -> Result<(), PipelineError> {
        self.manager.reconfigure_at(name, config, boundary)
    }

    fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn ProcessNode>,
        inputs: Vec<Option<InputSub>>,
    ) -> Result<(), PipelineError> {
        self.manager.restart_node(name, node, inputs)
    }

    fn progress(&self) -> Vec<(String, u64)> {
        self.manager.progress()
    }

    fn take_disconnected(&self) -> Vec<DisconnectEvent> {
        self.manager.take_disconnected()
    }

    fn take_failures(&mut self) -> Vec<NodeFailure> {
        self.manager.take_failures()
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

#[cfg(test)]
mod pipeline_tests {
    use platform_runtime::{WorkExecutorError, WorkExecutorTask, WorkTask};

    use super::*;

    struct RejectingWorkExecutor;

    impl WorkExecutor for RejectingWorkExecutor {
        fn available_parallelism(&self) -> usize {
            1
        }

        fn submit(&self, _task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
            Err(WorkExecutorError::Stopped)
        }
    }

    #[test]
    fn factory_preserves_supervision_start_failure() {
        let factory = PipelineAppManagerFactory::new(Arc::new(RejectingWorkExecutor));
        let error = match factory.create() {
            Ok(_) => panic!("runtime factory should reject unavailable supervision"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            PipelineError::WatchdogStart {
                source: WorkExecutorError::Stopped,
            }
        );
    }
}
