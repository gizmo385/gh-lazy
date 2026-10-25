//! The JSONL request log (`FAKE_GH_LOG`).
//!
//! Every invocation appends one line. The Python app runs several `gh` processes at once, so the file is held under
//! an exclusive lock from the read that `when` conditions need until the append is done.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fixture::{Accept, LoggedRequestMatcher, Query};

/// One line of the request log. Field order is the order they're written in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// The 1-based line number at the moment of writing. Set by [`LockedLog::append`].
    pub seq: u64,
    /// The arguments fake-gh received, without the program name.
    pub argv: Vec<String>,
    /// `api`, `auth status`, `auth token`, `version` or `pr diff`. `null` when argv couldn't be parsed.
    pub command: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub query: Option<Query>,
    /// The normalized accept name, e.g. `json` or `diff`.
    pub accept: Option<String>,
    /// The `--cache` duration, if given.
    pub cache: Option<String>,
    /// The last `Cache-Control` header, if given.
    pub cache_control: Option<String>,
    /// The request body: JSON when it parsed, a string otherwise.
    pub body: Option<Value>,
    /// The fixture served, relative to the parent of its set directory, with `/` separators.
    pub fixture: Option<String>,
    pub status: Option<u16>,
    /// No fixture matched; fake-gh answered with a 404.
    pub miss: bool,
    /// Why fake-gh refused the invocation (unsupported flag or command, fixture load error).
    pub error: Option<String>,
}

impl LogEntry {
    /// Whether this line records a request that fake-gh answered from a fixture.
    pub fn is_served(&self) -> bool {
        !self.miss && self.error.is_none() && self.fixture.is_some()
    }

    fn matches(&self, m: &LoggedRequestMatcher) -> bool {
        let empty = Query::new();
        self.is_served()
            && self.method.as_deref() == Some(m.method.as_str())
            && self.path.as_ref() == Some(&m.path)
            && m.query
                .as_ref()
                .is_none_or(|q| self.query.as_ref().unwrap_or(&empty) == q)
            && m.accept
                .as_ref()
                .is_none_or(|a| self.accept.as_deref().map(Accept::normalize).unwrap_or_default() == *a)
            && m.body.as_ref().is_none_or(|b| self.body.as_ref() == Some(b))
    }
}

/// The log's contents as `when` conditions see them.
#[derive(Debug, Clone, Default)]
pub struct RequestLog {
    enabled: bool,
    entries: Vec<LogEntry>,
}

impl RequestLog {
    /// No log configured: every `when` condition is false.
    pub fn disabled() -> Self {
        RequestLog::default()
    }

    /// A log holding these entries.
    pub fn from_entries(entries: Vec<LogEntry>) -> Self {
        RequestLog { enabled: true, entries }
    }

    /// Parses JSONL text. Lines that don't parse are skipped.
    pub fn parse(text: &str) -> Self {
        RequestLog::from_entries(text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
    }

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// Whether some served request in the log matches `m`.
    pub fn has_served(&self, m: &LoggedRequestMatcher) -> bool {
        self.enabled && self.entries.iter().any(|e| e.matches(m))
    }
}

/// The log file, held under an exclusive lock until dropped or [`LockedLog::append`]ed to.
pub struct LockedLog {
    file: File,
    lines: u64,
    log: RequestLog,
}

impl LockedLog {
    /// Opens (creating if needed) and locks the log, then reads it.
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = OpenOptions::new().read(true).append(true).create(true).open(path)?;
        file.lock()?;
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        let lines = text.lines().filter(|l| !l.trim().is_empty()).count() as u64;
        Ok(LockedLog {
            file,
            lines,
            log: RequestLog::parse(&text),
        })
    }

    pub fn log(&self) -> &RequestLog {
        &self.log
    }

    /// Appends `entry` with the next `seq`, then releases the lock.
    pub fn append(mut self, mut entry: LogEntry) -> io::Result<u64> {
        entry.seq = self.lines + 1;
        let mut line = serde_json::to_vec(&entry).map_err(io::Error::other)?;
        line.push(b'\n');
        // One write call, so a reader that ignores the lock still never sees half a line from us.
        self.file.write_all(&line)?;
        self.file.flush()?;
        self.file.unlock()?;
        Ok(entry.seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::query_from_pairs;

    fn served(method: &str, path: &str) -> LogEntry {
        LogEntry {
            command: Some("api".into()),
            method: Some(method.into()),
            path: Some(path.into()),
            fixture: Some("base/x/PUT.json".into()),
            status: Some(200),
            ..LogEntry::default()
        }
    }

    fn matcher(method: &str, path: &str) -> LoggedRequestMatcher {
        LoggedRequestMatcher {
            method: method.into(),
            path: path.into(),
            query: None,
            accept: None,
            body: None,
        }
    }

    #[test]
    fn disabled_log_never_matches() {
        assert!(!RequestLog::disabled().has_served(&matcher("GET", "/x")));
    }

    #[test]
    fn misses_and_errors_are_not_served() {
        let mut miss = served("PUT", "/x");
        miss.miss = true;
        miss.fixture = None;
        let mut err = served("PUT", "/x");
        err.error = Some("boom".into());
        let log = RequestLog::from_entries(vec![miss, err]);
        assert!(!log.has_served(&matcher("PUT", "/x")));
        let log = RequestLog::from_entries(vec![served("PUT", "/x")]);
        assert!(log.has_served(&matcher("PUT", "/x")));
        assert!(!log.has_served(&matcher("GET", "/x")));
    }

    #[test]
    fn optional_matcher_fields_are_compared() {
        let mut e = served("GET", "/x");
        e.query = Some(query_from_pairs([("page", "1")]));
        e.accept = Some("diff".into());
        let log = RequestLog::from_entries(vec![e]);
        let mut m = matcher("GET", "/x");
        m.query = Some(query_from_pairs([("page", "1")]));
        m.accept = Some(Accept::Diff);
        assert!(log.has_served(&m));
        m.query = Some(query_from_pairs([("page", "2")]));
        assert!(!log.has_served(&m));
    }
}
