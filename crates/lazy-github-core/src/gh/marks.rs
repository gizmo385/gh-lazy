//! Which cached reads the app's own writes made stale.
//!
//! `gh api --cache <ttl>` serves a stored response for as long as the TTL allows, so a read straight after a write
//! (a new comment, a merge) could show the copy from before it. A write declares the reads it makes stale as
//! [`CacheScope`]s, and Refresh from GitHub marks [`CacheScope::All`]; [`CacheMarks`] remembers them, and the runner refreshes the next matching read with
//! `--cache 1ns`, which always fetches and overwrites gh's stored copy. Marks live in memory only.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::request::CacheScope;

/// One cached read, as far as the marks are concerned: its path with the query, and its `Accept` value. gh's own key
/// also has the method (always `GET` here) and the token (the same for every request of one runner).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey {
    path_with_query: String,
    accept: String,
}

impl CacheKey {
    /// The key of a read of `path_with_query` (such as `/repos/o/r/issues?state=all`) with `accept`.
    pub(crate) fn new(path_with_query: impl Into<String>, accept: impl Into<String>) -> Self {
        Self {
            path_with_query: path_with_query.into(),
            accept: accept.into(),
        }
    }

    /// The path without its query.
    fn path(&self) -> &str {
        self.path_with_query
            .split_once('?')
            .map_or(self.path_with_query.as_str(), |(path, _)| path)
    }
}

impl CacheScope {
    /// Whether a read of `path` (no query) falls in this scope. Owner and repo are case-insensitive on GitHub, and
    /// `comments_url` carries GitHub's casing while the builders use the repository's, so the comparison ignores
    /// ASCII case. A trailing `/` on either side doesn't count.
    fn covers(&self, path: &str) -> bool {
        let path = path.trim_end_matches('/');
        match self {
            CacheScope::All => true,
            CacheScope::Path(scope) => path.eq_ignore_ascii_case(scope.trim_end_matches('/')),
            CacheScope::Subtree(scope) => {
                let scope = scope.trim_end_matches('/');
                match path.split_at_checked(scope.len()) {
                    Some((prefix, rest)) => {
                        prefix.eq_ignore_ascii_case(scope) && (rest.is_empty() || rest.starts_with('/'))
                    }
                    None => false,
                }
            }
        }
    }
}

/// The marks the app's writes left, and when each read last started a refresh.
///
/// Every method takes the time as an argument, so tests drive the clock.
#[derive(Debug, Default)]
pub(crate) struct CacheMarks {
    /// Each mark with the time its write finished, oldest first.
    marks: Vec<(CacheScope, Instant)>,
    /// When the last successful refresh of each key started.
    refreshed: HashMap<CacheKey, Instant>,
    /// How long a mark can matter: the longest cache TTL configured. A read with a shorter TTL ignores older marks on
    /// its own; this only bounds what is kept.
    retention: Duration,
}

impl CacheMarks {
    /// Keeps marks for `retention`, the longest cache TTL now configured. Applies from the next [`Self::mark`].
    pub(crate) fn set_retention(&mut self, retention: Duration) {
        self.retention = retention;
    }

    /// Records that a write finished at `at`, making every read in `scopes` stale. Drops the marks older than the
    /// retention, and the refresh records older than the oldest mark left.
    pub(crate) fn mark(&mut self, scopes: &[CacheScope], at: Instant) {
        let retention = self.retention;
        self.marks
            .retain(|(_, marked)| at.saturating_duration_since(*marked) < retention);
        self.marks.extend(scopes.iter().cloned().map(|scope| (scope, at)));
        match self.marks.iter().map(|(_, marked)| *marked).min() {
            Some(oldest) => self.refreshed.retain(|_, started| *started >= oldest),
            None => self.refreshed.clear(),
        }
    }

    /// Whether a read of `key` with cache TTL `ttl`, starting `now`, must refresh: some mark covers its path, is
    /// younger than `ttl` (gh won't serve an entry older than that anyway) and isn't older than the key's last
    /// refresh start (a read that started before the write can't have seen it).
    pub(crate) fn needs_refresh(&self, key: &CacheKey, ttl: Duration, now: Instant) -> bool {
        let path = key.path();
        let last = self.refreshed.get(key);
        self.marks.iter().any(|(scope, marked)| {
            now.saturating_duration_since(*marked) < ttl
                && last.is_none_or(|started| marked >= started)
                && scope.covers(path)
        })
    }

    /// Records that a refresh of `key`, started at `started`, succeeded: it clears the marks from before `started`.
    pub(crate) fn refreshed(&mut self, key: CacheKey, started: Instant) {
        let entry = self.refreshed.entry(key).or_insert(started);
        if started > *entry {
            *entry = started;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_mins(10);

    fn key(path: &str) -> CacheKey {
        CacheKey::new(path, "application/vnd.github+json")
    }

    fn item(n: u64) -> Vec<CacheScope> {
        vec![
            CacheScope::Subtree(format!("/repos/o/r/issues/{n}")),
            CacheScope::Subtree(format!("/repos/o/r/pulls/{n}")),
        ]
    }

    fn marked(scopes: &[CacheScope], at: Instant) -> CacheMarks {
        let mut marks = CacheMarks::default();
        marks.set_retention(TTL);
        marks.mark(scopes, at);
        marks
    }

    #[test]
    fn a_subtree_covers_its_path_and_below_but_not_a_longer_number() {
        let t0 = Instant::now();
        let marks = marked(&item(5), t0);
        for path in [
            "/repos/o/r/issues/5",
            "/repos/o/r/issues/5/",
            "/repos/o/r/issues/5/comments",
            "/repos/o/r/issues/5/reactions",
            "/repos/o/r/pulls/5/reviews/7/comments",
            "/repos/o/r/issues/5/comments?page=2",
        ] {
            assert!(marks.needs_refresh(&key(path), TTL, t0), "{path}");
        }
        for path in [
            "/repos/o/r/issues/50",
            "/repos/o/r/issues/4",
            "/repos/o/r/issues",
            "/repos/o/r/issues/comments/5/reactions",
            "/repos/o/other/issues/5",
        ] {
            assert!(!marks.needs_refresh(&key(path), TTL, t0), "{path}");
        }
    }

    #[test]
    fn a_path_covers_any_query_but_nothing_below() {
        let t0 = Instant::now();
        let marks = marked(&[CacheScope::Path("/repos/o/r/issues".into())], t0);
        assert!(marks.needs_refresh(&key("/repos/o/r/issues?state=all&page=1&per_page=30"), TTL, t0));
        assert!(marks.needs_refresh(&key("/repos/o/r/issues"), TTL, t0));
        assert!(!marks.needs_refresh(&key("/repos/o/r/issues/5"), TTL, t0));
        assert!(!marks.needs_refresh(&key("/repos/o/r/issues5"), TTL, t0));
    }

    #[test]
    fn matching_ignores_owner_and_repo_case() {
        let t0 = Instant::now();
        let marks = marked(&item(5), t0);
        assert!(marks.needs_refresh(&key("/repos/O/R/issues/5/comments"), TTL, t0));
        let marks = marked(&[CacheScope::Subtree("/repos/Octo/Lazy/pulls/5".into())], t0);
        assert!(marks.needs_refresh(&key("/repos/octo/lazy/pulls/5/reviews"), TTL, t0));
    }

    #[test]
    fn a_mark_refreshes_each_key_once() {
        let t0 = Instant::now();
        let mut marks = marked(&item(5), t0);
        let comments = key("/repos/o/r/issues/5/comments");
        let reviews = key("/repos/o/r/pulls/5/reviews");
        let t1 = t0 + Duration::from_secs(1);
        assert!(marks.needs_refresh(&comments, TTL, t1));
        marks.refreshed(comments.clone(), t1);
        assert!(!marks.needs_refresh(&comments, TTL, t1 + Duration::from_secs(1)));
        // Another key under the same mark still refreshes.
        assert!(marks.needs_refresh(&reviews, TTL, t1 + Duration::from_secs(1)));
        // The same path with another `Accept` is another gh cache entry.
        let diff = CacheKey::new("/repos/o/r/pulls/5", "application/vnd.github.diff");
        assert!(marks.needs_refresh(&diff, TTL, t1));
    }

    #[test]
    fn a_refresh_started_before_the_mark_does_not_clear_it() {
        let t0 = Instant::now();
        let comments = key("/repos/o/r/issues/5/comments");
        let mut marks = CacheMarks::default();
        marks.set_retention(TTL);
        let t1 = t0 + Duration::from_secs(1);
        marks.mark(&item(5), t1);
        // A read that started at t0, before the write finished, comes back at t2.
        marks.refreshed(comments.clone(), t0);
        assert!(marks.needs_refresh(&comments, TTL, t1 + Duration::from_secs(1)));
        // A later mark after a refresh is honoured too.
        let t3 = t1 + Duration::from_secs(5);
        marks.refreshed(comments.clone(), t3);
        assert!(!marks.needs_refresh(&comments, TTL, t3));
        marks.mark(&item(5), t3 + Duration::from_secs(1));
        assert!(marks.needs_refresh(&comments, TTL, t3 + Duration::from_secs(2)));
    }

    #[test]
    fn a_failed_refresh_keeps_the_mark() {
        let t0 = Instant::now();
        let marks = marked(&item(5), t0);
        let comments = key("/repos/o/r/issues/5/comments");
        // A failed refresh records nothing, so the next read asks again.
        assert!(marks.needs_refresh(&comments, TTL, t0 + Duration::from_secs(1)));
        assert!(marks.needs_refresh(&comments, TTL, t0 + Duration::from_secs(2)));
    }

    #[test]
    fn a_mark_older_than_the_reads_ttl_is_ignored() {
        let t0 = Instant::now();
        let marks = marked(&item(5), t0);
        let comments = key("/repos/o/r/issues/5/comments");
        assert!(marks.needs_refresh(&comments, TTL, t0 + Duration::from_secs(599)));
        assert!(!marks.needs_refresh(&comments, TTL, t0 + TTL));
        let short = Duration::from_secs(30);
        assert!(!marks.needs_refresh(&comments, short, t0 + Duration::from_secs(31)));
        // A zero TTL never refreshes: the read isn't cached at all.
        assert!(!marks.needs_refresh(&comments, Duration::ZERO, t0));
    }

    #[test]
    fn all_covers_every_path() {
        let t0 = Instant::now();
        let marks = marked(&[CacheScope::All], t0);
        for path in ["/user/repos?page=1", "/repos/o/r/readme", "/notifications", "/", ""] {
            assert!(marks.needs_refresh(&key(path), TTL, t0), "{path}");
        }
    }

    #[test]
    fn after_an_all_mark_each_key_refreshes_once() {
        let t0 = Instant::now();
        let mut marks = marked(&[CacheScope::All], t0);
        let readme = key("/repos/o/r/readme");
        let repos = key("/user/repos?page=1");
        for read in [&readme, &repos] {
            let t1 = t0 + Duration::from_secs(1);
            assert!(marks.needs_refresh(read, TTL, t1));
            marks.refreshed(read.clone(), t1);
            assert!(!marks.needs_refresh(read, TTL, t1 + Duration::from_secs(1)));
        }
    }

    #[test]
    fn a_read_started_before_an_all_mark_does_not_clear_it() {
        let t0 = Instant::now();
        let readme = key("/repos/o/r/readme");
        let t1 = t0 + Duration::from_secs(1);
        let mut marks = marked(&[CacheScope::All], t1);
        marks.refreshed(readme.clone(), t0);
        assert!(marks.needs_refresh(&readme, TTL, t1 + Duration::from_secs(1)));
    }

    #[test]
    fn an_all_mark_is_ignored_past_the_reads_ttl() {
        let t0 = Instant::now();
        let marks = marked(&[CacheScope::All], t0);
        let readme = key("/repos/o/r/readme");
        let short = Duration::from_mins(1);
        assert!(marks.needs_refresh(&readme, short, t0 + Duration::from_secs(59)));
        assert!(!marks.needs_refresh(&readme, short, t0 + short));
        assert!(!marks.needs_refresh(&readme, Duration::ZERO, t0));
    }

    #[test]
    fn pruning_drops_old_marks_and_the_refresh_records_before_the_oldest_left() {
        let t0 = Instant::now();
        let mut marks = marked(&item(5), t0);
        let old = key("/repos/o/r/issues/5/comments");
        marks.refreshed(old.clone(), t0 + Duration::from_secs(1));
        assert_eq!(marks.marks.len(), 2);

        // Within the retention, nothing goes.
        let t1 = t0 + Duration::from_mins(1);
        marks.mark(&[CacheScope::Path("/repos/o/r/issues".into())], t1);
        assert_eq!(marks.marks.len(), 3);
        assert!(marks.refreshed.contains_key(&old));

        // Past it, the first write's marks go, and so does the refresh record from before the oldest mark left.
        let t2 = t0 + TTL + Duration::from_secs(1);
        marks.mark(&item(7), t2);
        assert_eq!(marks.marks.len(), 3);
        assert!(marks.marks.iter().all(|(_, at)| *at >= t1));
        assert!(!marks.refreshed.contains_key(&old));

        // A later refresh record stays.
        let list = key("/repos/o/r/issues?state=all");
        marks.refreshed(list.clone(), t2);
        marks.mark(&[], t2);
        assert!(marks.refreshed.contains_key(&list));

        // With every mark gone, the records go too.
        marks.mark(&[], t2 + TTL + TTL);
        assert!(marks.marks.is_empty());
        assert!(marks.refreshed.is_empty());
    }
}
