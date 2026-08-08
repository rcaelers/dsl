use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use platform_artifacts::ArtifactRepository;
use platform_runtime::{
    WorkExecutor, WorkExecutorError, WorkExecutorTask, WorkTask, WorkerKernelRegistry,
    WorkerOperationExecutor,
};

use super::native_artifact_repository::NativeArtifactRepository;
use super::native_worker::NativeWorkerOperationExecutor;
use crate::WorkerAdapterError;

/// Opens the native durable artifact repository for an application.
pub fn native_artifact_repository(application_id: &str) -> Arc<dyn ArtifactRepository> {
    Arc::new(NativeArtifactRepository::new(
        derived_cache_directory(application_id).join("artifacts"),
    ))
}

/// Creates the native bounded executor for finite processing work.
pub fn native_work_executor() -> Arc<dyn WorkExecutor> {
    Arc::new(NativeWorkExecutor::new())
}

/// Creates the native worker-operation executor.
pub fn native_worker_operation_executor(
    kernels: WorkerKernelRegistry,
) -> Result<Rc<dyn WorkerOperationExecutor>, WorkerAdapterError> {
    NativeWorkerOperationExecutor::new(kernels)
        .map(|executor| Rc::new(executor) as Rc<dyn WorkerOperationExecutor>)
}

struct NativeWorkExecutor {
    sender: crossbeam_channel::Sender<WorkExecutorTask>,
    workers: usize,
}

impl NativeWorkExecutor {
    fn new() -> Self {
        let workers = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            // An index preparation task can submit bounded block work to the
            // same host executor. Keep one worker available for that nested
            // work even on single-core hosts.
            .clamp(2, 32);
        let (sender, receiver) = crossbeam_channel::bounded(workers * 4);
        for index in 0..workers {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(format!("processing-work-{index}"))
                .spawn(move || run_work_executor_worker(receiver))
                .expect("failed to start processing work executor");
        }
        Self { sender, workers }
    }
}

impl WorkExecutor for NativeWorkExecutor {
    fn available_parallelism(&self) -> usize {
        self.workers
    }

    fn supports_long_running_tasks(&self) -> bool {
        true
    }

    fn idle(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task_completed = Arc::clone(&completed);
        let (completion_sender, completion_receiver) = crossbeam_channel::bounded(1);
        self.sender
            .try_send(Box::new(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                task_completed.store(true, Ordering::Release);
                let _ = completion_sender.send(());
            }))
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => WorkExecutorError::QueueFull,
                crossbeam_channel::TrySendError::Disconnected(_) => WorkExecutorError::Stopped,
            })?;
        Ok(Box::new(NativeWorkTask {
            completed,
            completion_receiver,
        }))
    }

    fn submit_long_running(
        &self,
        task: WorkExecutorTask,
    ) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
        spawn_runtime_task(task)
    }
}

struct NativeWorkTask {
    completed: Arc<std::sync::atomic::AtomicBool>,
    completion_receiver: crossbeam_channel::Receiver<()>,
}

fn spawn_runtime_task(task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, WorkExecutorError> {
    let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let task_completed = Arc::clone(&completed);
    let (completion_sender, completion_receiver) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("processing-runtime".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
            task_completed.store(true, Ordering::Release);
            let _ = completion_sender.send(());
        })
        .map_err(|error| WorkExecutorError::TaskStart {
            message: error.to_string(),
        })?;
    Ok(Box::new(NativeWorkTask {
        completed,
        completion_receiver,
    }))
}

impl WorkTask for NativeWorkTask {
    fn is_finished(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    fn wait(self: Box<Self>) {
        let _ = self.completion_receiver.recv();
    }
}

fn run_work_executor_worker(receiver: crossbeam_channel::Receiver<WorkExecutorTask>) {
    while let Ok(task) = receiver.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
    }
}

fn derived_cache_directory(application_id: &str) -> PathBuf {
    application_cache_directory(application_id).join("derived")
}

fn application_cache_directory(application_id: &str) -> PathBuf {
    std::cfg_select! {
        target_os = "macos" => std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| application_directory(home.join("Library").join("Caches"), application_id))
            .unwrap_or_else(|| application_directory(std::env::temp_dir(), application_id)),
        target_os = "windows" => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|directory| application_directory(directory, application_id))
            .unwrap_or_else(|| application_directory(std::env::temp_dir(), application_id)),
        _ => std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".cache"))
            })
            .map(|directory| application_directory(directory, application_id))
            .unwrap_or_else(|| application_directory(std::env::temp_dir(), application_id)),
    }
}

fn application_directory(parent: PathBuf, application_id: &str) -> PathBuf {
    parent.join(application_id)
}

#[cfg(test)]
mod native_tests {
    use platform_runtime::WorkExecutor;

    use super::{NativeWorkExecutor, application_directory};

    #[test]
    fn native_work_executor_runs_submitted_work() {
        let executor = NativeWorkExecutor::new();
        let (sender, receiver) = std::sync::mpsc::channel();

        executor
            .submit(Box::new(move || sender.send(42).unwrap()))
            .unwrap();

        assert!(executor.available_parallelism() >= 1);
        assert_eq!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap(),
            42
        );
    }

    #[test]
    fn native_cache_directories_use_the_application_identifier() {
        let parent = tempfile::tempdir().unwrap();

        assert_eq!(
            application_directory(parent.path().to_owned(), "example-app"),
            parent.path().join("example-app")
        );
    }
}
