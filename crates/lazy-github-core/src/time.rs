//! Date formats the app shows, and the fixed wall clock of parity runs.

use jiff::Timestamp;

/// The environment variable that, in a debug build, freezes the app's wall clock at this RFC 3339 instant (for
/// example `2026-01-01T00:00:00Z`): what the command log, the log file and screenshot names show. The parity harness
/// sets it so those screens are the same on every run. Timers keep the real monotonic clock. Release builds ignore it.
pub const FIXED_TIME_ENV: &str = "LAZY_GITHUB_FIXED_TIME";

/// The instant [`FIXED_TIME_ENV`] freezes the wall clock at, in a debug build when it's set to a valid RFC 3339
/// timestamp; `None` otherwise.
pub fn fixed_time_from_env() -> Option<Timestamp> {
    fixed_time(cfg!(debug_assertions), std::env::var(FIXED_TIME_ENV).ok().as_deref())
}

/// `value` (a [`FIXED_TIME_ENV`] value) as an instant, when `debug` and it parses.
fn fixed_time(debug: bool, value: Option<&str>) -> Option<Timestamp> {
    if !debug {
        return None;
    }
    value.and_then(|v| v.trim().parse().ok())
}

/// `ts` as Python's `datetime.strftime("%c")` under the C locale, in UTC: `Sun Feb  1 08:30:00 2026`, the day of the
/// month padded with a space. Python shows API times in UTC (Py bug B26), and so does Rust.
pub fn c_format(ts: Timestamp) -> String {
    ts.strftime("%a %b %e %H:%M:%S %Y").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn a_one_digit_day_is_space_padded() {
        assert_eq!(c_format(at("2026-02-01T08:30:00Z")), "Sun Feb  1 08:30:00 2026");
    }

    #[test]
    fn the_fixed_time_is_read_only_in_debug_builds() {
        let value = Some("2026-01-01T00:00:00Z");
        assert_eq!(fixed_time(true, value), Some(at("2026-01-01T00:00:00Z")));
        assert_eq!(fixed_time(false, value), None);
        assert_eq!(fixed_time(true, None), None);
        assert_eq!(fixed_time(true, Some("yesterday")), None);
    }

    #[test]
    fn a_two_digit_day_is_not() {
        assert_eq!(c_format(at("2026-01-12T16:20:00Z")), "Mon Jan 12 16:20:00 2026");
        assert_eq!(c_format(at("2026-01-10T14:00:00+05:00")), "Sat Jan 10 09:00:00 2026");
    }
}
