use logic_analyzer_graph_capabilities::node_support::NodeBuildContext;
use signal_derived::{
    DecodedBlockCacheHandle, DerivedDataRetention, DerivedLanes, PersistentStoreConfig,
    SamplingPointStore,
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

    fn decoded_block_cache(&self) -> DecodedBlockCacheHandle {
        DecodedBlockCacheHandle::default()
    }

    fn sampling_points(&self, _runtime_name: &str) -> Option<SamplingPointStore> {
        None
    }
}
