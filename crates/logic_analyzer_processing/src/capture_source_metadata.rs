use signal_processing::{ConfiguredAcquisition, IndexedCapturePresentation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSourceKind {
    File,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureSourceLifecycle {
    kind: CaptureSourceKind,
    preload: bool,
    cache: bool,
    index: bool,
}

impl CaptureSourceLifecycle {
    pub const fn new(kind: CaptureSourceKind, preload: bool, cache: bool, index: bool) -> Self {
        Self {
            kind,
            preload,
            cache,
            index,
        }
    }

    pub const fn kind(self) -> CaptureSourceKind {
        self.kind
    }

    pub const fn preload(self) -> bool {
        self.preload
    }

    pub const fn cache(self) -> bool {
        self.cache
    }

    pub const fn index(self) -> bool {
        self.index
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureSourceSignal {
    index: usize,
    name: String,
    initial: bool,
    transitions: Vec<(f64, bool)>,
}

impl CaptureSourceSignal {
    pub fn new(
        index: usize,
        name: impl Into<String>,
        initial: bool,
        transitions: Vec<(f64, bool)>,
    ) -> Self {
        Self {
            index,
            name: name.into(),
            initial,
            transitions,
        }
    }

    pub const fn index(&self) -> usize {
        self.index
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn initial(&self) -> bool {
        self.initial
    }

    pub fn transitions(&self) -> &[(f64, bool)] {
        &self.transitions
    }

    pub fn into_parts(self) -> (usize, String, bool, Vec<(f64, bool)>) {
        (self.index, self.name, self.initial, self.transitions)
    }
}

pub enum CaptureSourcePresentation {
    Channels(Vec<(usize, String)>),
    Indexed(IndexedCapturePresentation),
    InMemory {
        signals: Vec<CaptureSourceSignal>,
        duration_us: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSourceCacheIdentity {
    NotCapture,
    Dynamic,
    Stable([u8; 32]),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureSourceRuntimeCapabilities {
    live_acquisition: bool,
}

impl CaptureSourceRuntimeCapabilities {
    pub const fn new(live_acquisition: bool) -> Self {
        Self { live_acquisition }
    }

    pub const fn live_acquisition(self) -> bool {
        self.live_acquisition
    }
}

/// Lazy, platform-neutral metadata supplied by a concrete source factory.
pub trait CaptureSourceMetadata: Send + Sync {
    fn lifecycle(&self) -> CaptureSourceLifecycle;
    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String>;
    fn cache_identity(&self) -> CaptureSourceCacheIdentity;
    fn channel_names(&self) -> Result<Option<Vec<String>>, String>;
    fn runtime_capabilities(&self) -> CaptureSourceRuntimeCapabilities {
        CaptureSourceRuntimeCapabilities::default()
    }
    fn configured_acquisition(&self) -> Result<Option<Box<dyn ConfiguredAcquisition>>, String> {
        Ok(None)
    }
}
