use signal_processing::ProcessNode;

/// A runtime process together with platform-neutral construction metadata.
///
/// Factory users consume the process while higher-level adapters translate the
/// metadata into their own contracts. The metadata type is deliberately
/// generic so platform backends do not leak through the facade.
pub struct ProcessNodeConstruction<M = ()> {
    process: Box<dyn ProcessNode>,
    metadata: M,
}

impl<M> ProcessNodeConstruction<M> {
    /// Couples a constructed process node with its source metadata.
    ///
    /// # Parameters
    /// - `process`: Input consumed by this operation.
    /// - `metadata`: Input consumed by this operation.
    pub fn new(process: Box<dyn ProcessNode>, metadata: M) -> Self {
        Self { process, metadata }
    }

    /// Returns the source metadata retained with this construction.
    pub fn metadata(&self) -> &M {
        &self.metadata
    }

    /// Consumes this value and returns process.
    pub fn into_process(self) -> Box<dyn ProcessNode> {
        self.process
    }

    /// Consumes this value and returns parts.
    pub fn into_parts(self) -> (Box<dyn ProcessNode>, M) {
        (self.process, self.metadata)
    }
}

#[cfg(test)]
mod process_node_construction_tests {
    use signal_processing::{InputPort, OutputPort, WorkResult};

    use super::*;

    struct TestProcess;

    impl ProcessNode for TestProcess {
        fn name(&self) -> &str {
            "test"
        }

        fn num_inputs(&self) -> usize {
            0
        }

        fn num_outputs(&self) -> usize {
            0
        }

        fn work(&mut self, _inputs: &[InputPort], _outputs: &[OutputPort]) -> WorkResult<usize> {
            Ok(0)
        }
    }

    #[test]
    fn construction_keeps_process_and_metadata_together() {
        let construction = ProcessNodeConstruction::new(
            Box::new(TestProcess) as Box<dyn ProcessNode>,
            "portable metadata",
        );

        assert_eq!(construction.metadata(), &"portable metadata");
        let (process, metadata) = construction.into_parts();
        assert_eq!(process.name(), "test");
        assert_eq!(metadata, "portable metadata");
    }
}
