/// Failure while binding lowered presentation metadata to UI renderer inventories.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum PresentationBindingError {
    /// A subscribed payload did not provide its required default lane presentation.
    #[error("subscribed payload '{payload_kind}' has no default presentation")]
    MissingDefaultLanePresentation {
        /// Stable payload kind whose presentation metadata was incomplete.
        payload_kind: String,
    },
    /// A collected waveform lane referenced a renderer absent from the viewer inventory.
    #[error("collected lane '{lane}' references unknown renderer '{renderer}'")]
    UnknownLaneRenderer {
        /// Collected lane requesting the renderer.
        lane: String,
        /// Stable renderer key that could not be resolved.
        renderer: String,
    },
    /// A decoder-table column referenced a renderer absent from the viewer inventory.
    #[error("decoder-table column '{column}' references unknown renderer '{renderer}'")]
    UnknownTableRenderer {
        /// Decoder-table column requesting the renderer.
        column: String,
        /// Stable renderer key that could not be resolved.
        renderer: String,
    },
}
