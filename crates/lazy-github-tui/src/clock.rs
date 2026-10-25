//! The app's only source of time, so snapshots and timer tests are deterministic.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// Where `App` and components get the time. Nothing else calls `Instant::now()` or `Zoned::now()`.
pub trait Clock: Send + Sync {
    /// Monotonic time, for deadlines and timers.
    fn now(&self) -> Instant;
    /// Wall-clock time in the local zone, for display.
    fn local_now(&self) -> jiff::Zoned;
}

/// The real clocks.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn local_now(&self) -> jiff::Zoned {
        jiff::Zoned::now()
    }
}

/// A clock that only moves when told to. Clones share the same time.
#[derive(Debug, Clone)]
pub struct FakeClock {
    inner: Arc<Mutex<FakeTime>>,
}

#[derive(Debug)]
struct FakeTime {
    now: Instant,
    local: jiff::Zoned,
}

impl FakeClock {
    /// A clock at 2026-10-25 12:00:00 UTC.
    pub fn new() -> Self {
        // 2026-10-25T12:00:00Z. In range, so `from_second` can't fail.
        let timestamp = jiff::Timestamp::from_second(1_792_929_600).unwrap_or(jiff::Timestamp::UNIX_EPOCH);
        Self::at(timestamp.to_zoned(jiff::tz::TimeZone::UTC))
    }

    /// A clock whose wall time starts at `local`.
    pub fn at(local: jiff::Zoned) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeTime {
                now: Instant::now(),
                local,
            })),
        }
    }

    /// Moves both clocks forward by `d`.
    pub fn advance(&self, d: Duration) {
        let mut time = self.lock();
        time.now += d;
        // A test advancing past the year 9999 is a bug in the test; leave the wall time where it is then.
        if let Ok(local) = time.local.checked_add(d) {
            time.local = local;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeTime> {
        // Nothing can panic while the lock is held, so a poisoned lock is still consistent.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.lock().now
    }

    fn local_now(&self) -> jiff::Zoned {
        self.lock().local.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_moves_only_when_advanced() {
        let clock = FakeClock::new();
        let start = clock.now();
        assert_eq!(clock.local_now().to_string(), "2026-10-25T12:00:00+00:00[UTC]");
        assert_eq!(clock.now(), start);
        clock.clone().advance(Duration::from_secs(90));
        assert_eq!(clock.now() - start, Duration::from_secs(90));
        assert_eq!(clock.local_now().to_string(), "2026-10-25T12:01:30+00:00[UTC]");
    }
}
