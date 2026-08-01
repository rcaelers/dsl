//! Host-facing construction of concrete node-runtime overrides.

use std::sync::Arc;

use logic_analyzer_graph_api::node::RuntimeBuilderOverride;
use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::DsLogicU3Pro16SourceFactory;

/// Returns the U3Pro16 builder override for one host-selected source factory.
pub fn u3pro16_runtime_builder_override(
    source_factory: Arc<dyn DsLogicU3Pro16SourceFactory>,
) -> RuntimeBuilderOverride {
    crate::nodes::sources::dslogic_u3pro16::runtime_builder_override(source_factory)
}
