use std::error::Error;
use std::sync::Arc;

use signal_capture::IndexedCapturePresentation;

use super::live_capture::ConfiguredAcquisition;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct CaptureSourceMetadataDiagnostic(String);

/// Failure categories exposed while inspecting or preparing capture-source metadata.
#[derive(Clone, Debug, thiserror::Error)]
pub enum CaptureSourceMetadataError {
    /// The source artifact or host resource could not be accessed.
    #[error("capture-source access failed: {0}")]
    Access(#[source] Arc<dyn Error + Send + Sync>),
    /// Available source bytes could not be decoded into capture metadata.
    #[error("capture-source metadata decoding failed: {0}")]
    Decode(#[source] Arc<dyn Error + Send + Sync>),
    /// The source could not create its configured live acquisition.
    #[error("capture-source acquisition configuration failed: {0}")]
    Acquisition(#[source] Arc<dyn Error + Send + Sync>),
}

impl PartialEq for CaptureSourceMetadataError {
    fn eq(&self, other: &Self) -> bool {
        // Discovery is polled, so independently reconstructed failures with the same category and
        // diagnostic must deduplicate while each value still retains its concrete typed source.
        match (self, other) {
            (Self::Access(left), Self::Access(right))
            | (Self::Decode(left), Self::Decode(right))
            | (Self::Acquisition(left), Self::Acquisition(right)) => {
                left.to_string() == right.to_string()
            }
            _ => false,
        }
    }
}

impl Eq for CaptureSourceMetadataError {}

impl CaptureSourceMetadataError {
    /// Classifies a typed source-access failure without discarding its concrete cause.
    pub fn access(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Access(Arc::new(error))
    }

    /// Classifies a source-access diagnostic whose provider exposes only display text.
    pub fn access_message(message: impl Into<String>) -> Self {
        Self::access(CaptureSourceMetadataDiagnostic(message.into()))
    }

    /// Classifies a typed metadata-decoding failure without discarding its concrete cause.
    pub fn decode(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Decode(Arc::new(error))
    }

    /// Classifies a typed acquisition-configuration failure without discarding its concrete cause.
    pub fn acquisition(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Acquisition(Arc::new(error))
    }
}

/// Distinguishes an imported capture from a source that acquires data live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSourceKind {
    /// A finite capture read from an existing file or other stored artifact.
    File,
    /// A source whose capture is acquired while the graph runs.
    Live,
}

/// Describes how the graph runtime must prepare and retain a capture source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureSourceLifecycle {
    kind: CaptureSourceKind,
    preload: bool,
    cache: bool,
    index: bool,
}

impl CaptureSourceLifecycle {
    /// Creates lifecycle requirements for a source.
    ///
    /// # Parameters
    /// - `kind`: Whether the source is an imported file or live acquisition.
    /// - `preload`: Whether the source must be read before the graph starts.
    /// - `cache`: Whether its samples may be retained in the capture cache.
    /// - `index`: Whether the runtime must build a waveform index for it.
    pub const fn new(kind: CaptureSourceKind, preload: bool, cache: bool, index: bool) -> Self {
        Self {
            kind,
            preload,
            cache,
            index,
        }
    }

    /// Returns whether the source is file-backed or live.
    pub const fn kind(self) -> CaptureSourceKind {
        self.kind
    }

    /// Returns whether the source must be fully prepared before execution.
    pub const fn preload(self) -> bool {
        self.preload
    }

    /// Returns whether the runtime should retain capture data for reuse.
    pub const fn cache(self) -> bool {
        self.cache
    }

    /// Returns whether the runtime should create an indexed waveform representation.
    pub const fn index(self) -> bool {
        self.index
    }
}

/// One digital signal used to present a small in-memory capture.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureSourceSignal {
    index: usize,
    name: String,
    initial: bool,
    transitions: Vec<(f64, bool)>,
}

impl CaptureSourceSignal {
    /// Creates an in-memory digital signal and its transitions.
    ///
    /// # Parameters
    /// - `index`: Stable zero-based channel position in the capture.
    /// - `name`: Human-readable channel name.
    /// - `initial`: Logic level before the first transition.
    /// - `transitions`: Time-ordered `(time_us, level)` changes after the initial level.
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

    /// Returns this signal's stable channel position.
    pub const fn index(&self) -> usize {
        self.index
    }

    /// Returns this signal's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the level before the first recorded transition.
    pub const fn initial(&self) -> bool {
        self.initial
    }

    /// Returns the time-ordered logic-level transitions, measured in microseconds.
    pub fn transitions(&self) -> &[(f64, bool)] {
        &self.transitions
    }

    /// Decomposes the signal into its channel metadata and transition sequence.
    pub fn into_parts(self) -> (usize, String, bool, Vec<(f64, bool)>) {
        (self.index, self.name, self.initial, self.transitions)
    }
}

/// Capture data available to the graph and viewer without opening the source again.
pub enum CaptureSourcePresentation {
    /// Channel identities only; samples will be supplied by a live acquisition.
    Channels(Vec<(usize, String)>),
    /// A durable indexed capture that the viewer can query by time range.
    Indexed(IndexedCapturePresentation),
    /// A finite capture represented directly by individual digital transitions.
    InMemory {
        /// Signals in stable channel order.
        signals: Vec<CaptureSourceSignal>,
        /// Capture end time in microseconds.
        duration_us: f64,
    },
}

/// Determines whether a source's cached derived data can be reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSourceCacheIdentity {
    /// This source does not produce a reusable finite capture.
    NotCapture,
    /// The source may produce different capture data each time it runs.
    Dynamic,
    /// A stable digest identifying immutable capture content.
    Stable([u8; 32]),
}

/// Optional runtime behavior advertised by a source implementation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureSourceRuntimeCapabilities {
    live_acquisition: bool,
}

impl CaptureSourceRuntimeCapabilities {
    /// Creates a capability set.
    ///
    /// # Parameters
    /// - `live_acquisition`: Whether the source can participate in interactive live acquisition.
    pub const fn new(live_acquisition: bool) -> Self {
        Self { live_acquisition }
    }

    /// Returns whether the source supports interactive live acquisition.
    pub const fn live_acquisition(self) -> bool {
        self.live_acquisition
    }
}

/// Lazy, platform-neutral metadata supplied by a concrete capture-source factory.
///
/// Implementations describe preparation, presentation, and cache behavior without
/// leaking host-specific source handles into graph compilation.
pub trait CaptureSourceMetadata: Send + Sync {
    /// Returns the preparation and retention requirements for this source.
    fn lifecycle(&self) -> CaptureSourceLifecycle;

    /// Returns available presentation data, or an error if it cannot be inspected.
    ///
    /// `Ok(None)` means that presentation data is not available until acquisition starts.
    fn presentation(&self)
    -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError>;

    /// Returns the cache-reuse identity for the source's capture content.
    fn cache_identity(&self) -> CaptureSourceCacheIdentity;

    /// Returns source channel names when they can be discovered without acquisition.
    ///
    /// `Ok(None)` means names are unavailable or will be supplied later by the source.
    fn channel_names(&self) -> Result<Option<Vec<String>>, CaptureSourceMetadataError>;

    /// Returns optional interactive-acquisition capabilities.
    ///
    /// The default declares no live-acquisition support.
    fn runtime_capabilities(&self) -> CaptureSourceRuntimeCapabilities {
        CaptureSourceRuntimeCapabilities::default()
    }
    /// Returns an acquisition configuration for a live source, when it has one.
    ///
    /// The default returns `Ok(None)`, which is appropriate for finite file sources.
    fn configured_acquisition(
        &self,
    ) -> Result<Option<Box<dyn ConfiguredAcquisition>>, CaptureSourceMetadataError> {
        Ok(None)
    }
}

#[cfg(test)]
mod capture_source_metadata_tests {
    use super::CaptureSourceMetadataError;

    #[derive(Debug, thiserror::Error)]
    #[error("controlled metadata failure")]
    struct ControlledMetadataFailure;

    #[test]
    fn typed_causes_remain_available_in_each_metadata_category() {
        assert!(matches!(
            CaptureSourceMetadataError::access(ControlledMetadataFailure),
            CaptureSourceMetadataError::Access(source)
                if source.downcast_ref::<ControlledMetadataFailure>().is_some()
        ));
        assert!(matches!(
            CaptureSourceMetadataError::decode(ControlledMetadataFailure),
            CaptureSourceMetadataError::Decode(source)
                if source.downcast_ref::<ControlledMetadataFailure>().is_some()
        ));
        assert!(matches!(
            CaptureSourceMetadataError::acquisition(ControlledMetadataFailure),
            CaptureSourceMetadataError::Acquisition(source)
                if source.downcast_ref::<ControlledMetadataFailure>().is_some()
        ));
    }

    #[test]
    fn string_only_providers_are_explicitly_adapted_at_the_facade() {
        let error = CaptureSourceMetadataError::access_message("browser registry lookup failed");

        assert_eq!(
            error.to_string(),
            "capture-source access failed: browser registry lookup failed"
        );
    }
}
