//! Generic immutable capture contracts and packed capture data.

mod implementation;

pub use implementation::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureIndex,
    CaptureIndexBuildProgress, CaptureIndexFactory, CaptureMetadata, CaptureSampledChannel,
    CaptureSampledWindow, CaptureSource, CaptureTransition, CaptureWaveformSegment,
    IndexedCapturePresentation, packed_bit,
};
