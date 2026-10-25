//! The startup decision: trust a recent auth check and go straight to the main screen, or run `gh auth status` first.
//! Python's `LazyGithub.authenticate_with_github`.

use jiff::SignedDuration;
use jiff::civil::DateTime;
use jiff::{RoundMode, Unit};

use crate::component::Cx;
use crate::effect::{GithubRequest, RequestId};

/// The blank screen shown until the first auth check answers. Python shows nothing until `on_ready` either.
#[derive(Debug, Default)]
pub struct StartupScreen {
    /// The startup `CheckAuth`, while it's in flight.
    pending: Option<RequestId>,
}

/// What [`StartupScreen::begin`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupDecision {
    /// The last check is recent enough: show the main screen without asking `gh`.
    Fresh,
    /// A `CheckAuth` was issued; its answer decides.
    Checking,
}

impl StartupScreen {
    /// Skips the check when `cache.auth_last_checked` is recent, otherwise issues `CheckAuth`.
    pub fn begin(&mut self, cx: &mut Cx<'_>) -> StartupDecision {
        let cache = &cx.config().cache;
        if auth_is_fresh(
            cache.auth_last_checked,
            cx.local_now().datetime(),
            cache.auth_cache_duration,
        ) {
            return StartupDecision::Fresh;
        }
        self.pending = Some(cx.request(GithubRequest::CheckAuth));
        StartupDecision::Checking
    }

    /// Whether `id` is the startup check. Claims it, so a repeated response isn't handled twice.
    pub fn take_pending(&mut self, id: RequestId) -> bool {
        if self.pending == Some(id) {
            self.pending = None;
            true
        } else {
            false
        }
    }
}

/// Whether an auth check made at `last` can still be trusted at `now`: less than `duration_secs` ago, strictly. A
/// check in the future counts as fresh, as Python's `(now - last).total_seconds() < duration` does.
pub fn auth_is_fresh(last: Option<DateTime>, now: DateTime, duration_secs: u64) -> bool {
    let Some(last) = last else { return false };
    let duration = SignedDuration::from_secs(i64::try_from(duration_secs).unwrap_or(i64::MAX));
    now.duration_since(last) < duration
}

/// `now` as Python's `datetime.now()` holds it: naive, truncated to microseconds. Saving and reloading it gives the
/// same value back.
pub fn naive_now(now: &jiff::Zoned) -> DateTime {
    let dt = now.datetime();
    // Rounding down to a microsecond can't leave jiff's range, so the fallback is never taken.
    dt.round(
        jiff::civil::DateTimeRound::new()
            .smallest(Unit::Microsecond)
            .mode(RoundMode::Trunc),
    )
    .unwrap_or(dt)
}

#[cfg(test)]
mod tests {
    use jiff::ToSpan;

    use super::*;

    const DAY: u64 = 86_400;

    fn now() -> DateTime {
        jiff::civil::date(2026, 10, 25).at(12, 0, 0, 0)
    }

    #[test]
    fn no_previous_check_is_not_fresh() {
        assert!(!auth_is_fresh(None, now(), DAY));
    }

    #[test]
    fn a_check_just_under_the_duration_ago_is_fresh() {
        assert!(auth_is_fresh(Some(now() - 86_399.seconds()), now(), DAY));
    }

    #[test]
    fn a_check_exactly_the_duration_ago_is_not_fresh() {
        assert!(!auth_is_fresh(Some(now() - 86_400.seconds()), now(), DAY));
    }

    #[test]
    fn a_check_in_the_future_is_fresh() {
        assert!(auth_is_fresh(Some(now() + 1.hour()), now(), DAY));
    }

    #[test]
    fn a_huge_duration_does_not_overflow() {
        assert!(auth_is_fresh(Some(now() - 1.hour()), now(), u64::MAX));
    }

    #[test]
    fn naive_now_drops_the_zone_and_sub_microseconds() {
        let zoned = now()
            .with()
            .subsec_nanosecond(123_456_789)
            .build()
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(-7)))
            .unwrap();
        assert_eq!(naive_now(&zoned).to_string(), "2026-10-25T12:00:00.123456");
    }
}
