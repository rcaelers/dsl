use web_time::{SystemTime, UNIX_EPOCH};

/// Supplies fixed-width wall-clock timestamps to persistent metadata.
///
/// Algorithms receive this capability through configuration so conformance
/// tests can make complete persistent generations deterministic without
/// changing the storage implementation under test.
pub trait UnixTimeSource: Send + Sync {
    fn now_unix_ns(&self) -> u64;
}

/// Portable wall-clock implementation used by default on every target.
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
