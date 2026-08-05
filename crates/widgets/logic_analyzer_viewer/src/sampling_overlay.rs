use signal_derived::SamplingPointStore;

/// Protocol-neutral presentation of sampling decisions already produced by
/// a processing node. Channel numbers identify the raw rows on which the
/// cached values are rendered; the viewer never interprets those channels.
#[derive(Debug, Clone)]
pub struct SamplingOverlay {
    /// Raw viewer channel carrying the sampling clock.
    pub clock_channel: usize,
    /// Raw viewer channels sampled at clock transitions.
    pub sampled_channels: Vec<usize>,
    /// Store used to recover sampling decisions.
    pub points: SamplingPointStore,
}
