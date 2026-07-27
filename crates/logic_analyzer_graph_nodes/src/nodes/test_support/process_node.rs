use signal_processing::{InputPort, OutputPort, ProcessNode, WorkResult};

pub(crate) struct TestProcessNode {
    name: String,
}

impl TestProcessNode {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl ProcessNode for TestProcessNode {
    fn name(&self) -> &str {
        &self.name
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
