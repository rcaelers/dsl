use signal_processing::SamplingPointStore;

/// Protocol-neutral presentation of sampling decisions already produced by
/// a processing node. Channel numbers identify the raw rows on which the
/// cached values are rendered; the viewer never interprets those channels.
#[derive(Debug, Clone)]
pub struct SamplingOverlay {
    pub clock_channel: usize,
    pub sampled_channels: Vec<usize>,
    pub points: SamplingPointStore,
}
