//! Platform-neutral system-activity leases and interruption observation.

use std::time::Duration;

use web_time::{Instant, SystemTime};

const SUSPEND_DETECTION_TOLERANCE: Duration = Duration::from_secs(5);

/// An interruption observed while a system-activity lease was active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemActivityInterruption {
    suspended_for: Duration,
}

impl SystemActivityInterruption {
    /// Creates an interruption from the host-observed suspended interval.
    pub fn new(suspended_for: Duration) -> Self {
        Self { suspended_for }
    }

    /// Returns the wall-clock interval not accounted for by active monotonic time.
    pub fn suspended_for(self) -> Duration {
        self.suspended_for
    }
}

/// One scoped request to keep host work active.
///
/// Dropping the lease releases any host inhibition. Consumers poll the lease
/// while work is active so hosts without inhibition can still report a
/// suspend/resume interruption.
pub trait SystemActivityLease {
    /// Reports whether the host successfully installed a sleep inhibitor.
    fn sleep_inhibited(&self) -> bool;

    /// Returns the next suspend/resume interruption, if one was observed.
    fn poll_interruption(&mut self) -> Option<SystemActivityInterruption>;
}

/// Host capability for scoped system activity.
pub trait SystemActivityManager: Send + Sync {
    /// Starts one activity lease with a human-readable host diagnostic.
    fn begin_activity(&self, reason: &str) -> Box<dyn SystemActivityLease>;
}

/// Portable observation-only manager used when host inhibition is unavailable.
#[derive(Default)]
pub struct ObservedSystemActivityManager;

impl SystemActivityManager for ObservedSystemActivityManager {
    fn begin_activity(&self, _reason: &str) -> Box<dyn SystemActivityLease> {
        Box::new(ObservedSystemActivityLease::new(false))
    }
}

/// Observation support shared by target-specific activity leases.
pub struct ObservedSystemActivityLease {
    sleep_inhibited: bool,
    previous: ActivityClockSample,
}

impl ObservedSystemActivityLease {
    /// Creates an observer and records whether its enclosing host lease inhibited sleep.
    pub fn new(sleep_inhibited: bool) -> Self {
        Self {
            sleep_inhibited,
            previous: ActivityClockSample::now(),
        }
    }

    fn observe(&mut self, current: ActivityClockSample) -> Option<SystemActivityInterruption> {
        let previous = self.previous;
        self.previous = current;
        let wall_elapsed = current.wall.duration_since(previous.wall).ok()?;
        let active_elapsed = current.active.saturating_duration_since(previous.active);
        let suspended_for = wall_elapsed.saturating_sub(active_elapsed);
        (suspended_for >= SUSPEND_DETECTION_TOLERANCE)
            .then_some(SystemActivityInterruption { suspended_for })
    }
}

impl SystemActivityLease for ObservedSystemActivityLease {
    fn sleep_inhibited(&self) -> bool {
        self.sleep_inhibited
    }

    fn poll_interruption(&mut self) -> Option<SystemActivityInterruption> {
        self.observe(ActivityClockSample::now())
    }
}

#[derive(Clone, Copy)]
struct ActivityClockSample {
    wall: SystemTime,
    active: Instant,
}

impl ActivityClockSample {
    fn now() -> Self {
        Self {
            wall: SystemTime::now(),
            active: Instant::now(),
        }
    }
}

#[cfg(test)]
mod system_activity_tests {
    use super::*;

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn observation_reports_only_wall_time_missing_from_active_time() {
        let start = ActivityClockSample::now();
        let mut lease = ObservedSystemActivityLease {
            sleep_inhibited: false,
            previous: start,
        };
        let ordinary_delay = ActivityClockSample {
            wall: start.wall + Duration::from_secs(30),
            active: start.active + Duration::from_secs(30),
        };
        assert_eq!(lease.observe(ordinary_delay), None);

        let resumed = ActivityClockSample {
            wall: ordinary_delay.wall + Duration::from_secs(65),
            active: ordinary_delay.active + Duration::from_secs(5),
        };
        assert_eq!(
            lease.observe(resumed),
            Some(SystemActivityInterruption {
                suspended_for: Duration::from_secs(60),
            })
        );
    }

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn observation_ignores_small_clock_skew_and_wall_clock_rollback() {
        let start = ActivityClockSample::now();
        let mut lease = ObservedSystemActivityLease {
            sleep_inhibited: true,
            previous: start,
        };
        assert!(lease.sleep_inhibited());

        let skewed = ActivityClockSample {
            wall: start.wall + Duration::from_secs(6),
            active: start.active + Duration::from_secs(2),
        };
        assert_eq!(lease.observe(skewed), None);
        let rolled_back = ActivityClockSample {
            wall: start.wall,
            active: skewed.active + Duration::from_secs(1),
        };
        assert_eq!(lease.observe(rolled_back), None);
    }
}
