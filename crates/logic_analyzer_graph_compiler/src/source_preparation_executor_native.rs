use std::sync::mpsc::{self, Receiver, TryRecvError};

use super::source_preparation_executor::{
    SourcePreparationExecutor, SourcePreparationResult, SourcePreparationTask,
    SourcePreparationTaskUpdate, SourcePreparationWork,
};

pub(crate) struct NativeSourcePreparationExecutor;

impl SourcePreparationExecutor for NativeSourcePreparationExecutor {
    fn submit(
        &self,
        work: SourcePreparationWork,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("capture-source-preparation".into())
            .spawn(move || {
                let _ = sender.send(work());
            })
            .map_err(|error| error.to_string())?;
        Ok(Box::new(NativeSourcePreparationTask { receiver }))
    }
}

struct NativeSourcePreparationTask {
    receiver: Receiver<SourcePreparationResult>,
}

impl SourcePreparationTask for NativeSourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate {
        match self.receiver.try_recv() {
            Ok(result) => SourcePreparationTaskUpdate::Complete(result),
            Err(TryRecvError::Empty) => SourcePreparationTaskUpdate::Pending,
            Err(TryRecvError::Disconnected) => SourcePreparationTaskUpdate::Disconnected,
        }
    }
}
