//! The app's only source of time, so snapshots and timer tests are deterministic.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use lazy_github_core::logging::{LogClock, system_clock};
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

/// The clocks the binary runs the app with: the real monotonic clock, and the wall clock either real or, in a debug
/// build with `lazy_github_core::time::FIXED_TIME_ENV` set, frozen at that instant, so parity runs show the same
/// timestamps every time. [`WallClock::log_clock`] gives the log file and the command log the same wall clock.
#[derive(Debug, Clone, Default)]
pub struct WallClock {
    frozen: Option<jiff::Zoned>,
}

impl WallClock {
    /// Frozen when the environment asks for it ([`lazy_github_core::time::fixed_time_from_env`]), real otherwise.
    pub fn from_env() -> Self {
        Self::frozen_at(lazy_github_core::time::fixed_time_from_env())
    }

    /// The wall clock frozen at `instant` in the local zone, or real with `None`.
    pub fn frozen_at(instant: Option<jiff::Timestamp>) -> Self {
        Self {
            frozen: instant.map(|ts| ts.to_zoned(jiff::tz::TimeZone::system())),
        }
    }

    /// The same wall clock for log timestamps.
    pub fn log_clock(&self) -> LogClock {
        match &self.frozen {
            Some(local) => {
                let at = local.datetime();
                Arc::new(move || at)
            }
            None => system_clock(),
        }
    }
}

impl Clock for WallClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn local_now(&self) -> jiff::Zoned {
        self.frozen.clone().unwrap_or_else(jiff::Zoned::now)
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
    fn a_frozen_wall_clock_and_its_log_clock_read_the_fixed_instant() {
        let instant: jiff::Timestamp = "2026-01-01T00:00:00Z".parse().unwrap();
        let clock = WallClock::frozen_at(Some(instant));
        let start = clock.now();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(clock.local_now().timestamp(), instant);
        assert!(clock.now() > start, "the monotonic clock still moves");
        let local = instant.to_zoned(jiff::tz::TimeZone::system()).datetime();
        assert_eq!((clock.log_clock())(), local);

        // Real without a fixed instant.
        let real = WallClock::frozen_at(None);
        assert!(real.local_now().timestamp() > instant);
        assert!((real.log_clock())() > local);
    }

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
