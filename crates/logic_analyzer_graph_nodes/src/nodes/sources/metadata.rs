use logic_analyzer_graph_capabilities::node_support::{
    CaptureCacheIdentity, CapturePresentation, CapturePresentationSignal, SourceDataLifecycle,
    SourceDataLifecycleKind,
};
use logic_analyzer_processing::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle,
    CaptureSourcePresentation,
};

pub(crate) fn lifecycle(metadata: CaptureSourceLifecycle) -> SourceDataLifecycle {
    let kind = match metadata.kind() {
        CaptureSourceKind::File => SourceDataLifecycleKind::File,
        CaptureSourceKind::Live => SourceDataLifecycleKind::Live,
    };
    SourceDataLifecycle::new(kind, metadata.preload(), metadata.cache(), metadata.index())
}

pub(crate) fn presentation(metadata: CaptureSourcePresentation) -> CapturePresentation {
    match metadata {
        CaptureSourcePresentation::Channels(channels) => CapturePresentation::Channels(channels),
        CaptureSourcePresentation::Indexed(indexed) => CapturePresentation::Indexed {
            identity: indexed.identity,
            factory: indexed.factory,
        },
        CaptureSourcePresentation::InMemory {
            signals,
            duration_us,
        } => CapturePresentation::InMemory {
            signals: signals
                .into_iter()
                .map(|signal| {
                    let (index, name, initial, transitions) = signal.into_parts();
                    CapturePresentationSignal {
                        index,
                        name,
                        initial,
                        transitions,
                    }
                })
                .collect(),
            duration_us,
        },
    }
}

pub(crate) fn cache_identity(metadata: CaptureSourceCacheIdentity) -> CaptureCacheIdentity {
    match metadata {
        CaptureSourceCacheIdentity::NotCapture => CaptureCacheIdentity::NotCapture,
        CaptureSourceCacheIdentity::Dynamic => CaptureCacheIdentity::Dynamic,
        CaptureSourceCacheIdentity::Stable(identity) => CaptureCacheIdentity::Stable(identity),
    }
}
