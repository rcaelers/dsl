use logic_analyzer_graph_api::node_support::NodeBuildContext;
use signal_processing::{
    DerivedDataRetention, DerivedLanes, PersistentStoreConfig, SamplingActivity,
};

#[derive(Default)]
pub(crate) struct TestNodeBuildContext {
    derived_lanes: DerivedLanes,
}

impl NodeBuildContext for TestNodeBuildContext {
    fn derived_lanes(&self) -> &DerivedLanes {
        &self.derived_lanes
    }

    fn derived_data_retention(&self) -> DerivedDataRetention {
        DerivedDataRetention::Unlimited
    }

    fn derived_word_cache(&self, _member: usize) -> Option<&PersistentStoreConfig> {
        None
    }

    fn sampling_activity(&self, _runtime_name: &str, _input: usize) -> Option<SamplingActivity> {
        None
    }
}
