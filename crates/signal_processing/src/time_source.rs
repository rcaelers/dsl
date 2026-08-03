use web_time::{SystemTime, UNIX_EPOCH};

/// Supplies fixed-width wall-clock timestamps to persistent metadata.
///
/// Algorithms receive this capability through configuration so conformance
/// tests can make complete persistent generations deterministic without
/// changing the storage implementation under test.
pub trait UnixTimeSource: Send + Sync {
    /// Returns the current wall-clock time as saturated nanoseconds since the Unix epoch.
    ///
    /// Values before the epoch and values that do not fit in `u64` are clamped
    /// by implementations rather than surfaced as errors, because metadata
    /// timestamps are best-effort ordering information.
    fn now_unix_ns(&self) -> u64;
}

/// Portable [`UnixTimeSource`] backed by the host wall clock.
///
/// This is the production default. Tests that need reproducible persisted
/// metadata inject a deterministic implementation of [`UnixTimeSource`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemUnixTimeSource;

impl UnixTimeSource for SystemUnixTimeSource {
    fn now_unix_ns(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64
    }
}
