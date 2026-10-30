//! `fake-browser`: the browser parity runs point `$BROWSER` at, and the log it keeps (`FAKE_BROWSER_LOG`).
//!
//! Both apps open links through `$BROWSER` (Python's `webbrowser`, the Rust app's `lazy_github_core::browser`), so
//! a run records what each app opened instead of starting a real browser. Each open appends one JSON line,
//! `{"url": "<url>"}`. At each capture the harness appends `{"capture": "<name>"}` to the same file, so the log says
//! what had been opened by every capture ([`opened_by`]).

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The environment variable naming the log.
pub const LOG_ENV: &str = "FAKE_BROWSER_LOG";

/// The file name of the fake-browser binary, next to fake-gh in the build dir and in the installed `bin/`.
pub fn file_name() -> String {
    format!("fake-browser{}", std::env::consts::EXE_SUFFIX)
}

/// One line of the log: an opened URL, or the harness's mark of a capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BrowserLine {
    /// `{"url": "..."}`: fake-browser was asked to open this.
    Opened {
        /// The URL, as the app passed it.
        url: String,
    },
    /// `{"capture": "..."}`: the harness took this capture here.
    Capture {
        /// The capture's name.
        capture: String,
    },
}

impl BrowserLine {
    /// The line as written, with its newline.
    pub fn to_line(&self) -> String {
        // Two string fields; serializing can't fail.
        serde_json::to_string(self).expect("a browser log line serializes") + "\n"
    }
}

/// Appends `line` to the log at `path` in one write, so lines from fake-browser and the harness never interleave.
///
/// # Errors
///
/// When the file can't be opened or written.
pub fn append(path: &Path, line: &BrowserLine) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.to_line().as_bytes())
}

/// The log's lines, skipping any that don't parse.
pub fn parse(text: &str) -> Vec<BrowserLine> {
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The URLs opened before the mark of `capture`, in order. Empty when the log has no mark for it (it was taken
/// before anything was logged).
pub fn opened_by(text: &str, capture: &str) -> Vec<String> {
    let lines = parse(text);
    let Some(mark) = lines
        .iter()
        .position(|line| matches!(line, BrowserLine::Capture { capture: c } if c == capture))
    else {
        return Vec::new();
    };
    lines[..mark]
        .iter()
        .filter_map(|line| match line {
            BrowserLine::Opened { url } => Some(url.clone()),
            BrowserLine::Capture { .. } => None,
        })
        .collect()
}

/// Whether the log opened anything at all.
pub fn has_opened(text: &str) -> bool {
    parse(text)
        .iter()
        .any(|line| matches!(line, BrowserLine::Opened { .. }))
}

/// Runs fake-browser with `args` (without the program name): logs `args[0]` as opened to `log`, prints nothing, and
/// returns the exit code, 0 unless the log can't be written. Without a log it opens nothing and still exits 0.
pub fn run(args: &[String], log: Option<&Path>, stderr: &mut impl Write) -> u8 {
    let Some(log) = log else { return 0 };
    let url = args.first().cloned().unwrap_or_default();
    match append(log, &BrowserLine::Opened { url }) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(stderr, "fake-browser: can't write {}: {e}", log.display());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_open_is_one_url_line() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("browser.jsonl");
        let mut stderr = Vec::new();
        assert_eq!(
            run(&["https://github.com/parity-bot".into()], Some(&log), &mut stderr),
            0
        );
        assert_eq!(run(&["https://example.com/a b".into()], Some(&log), &mut stderr), 0);
        assert!(stderr.is_empty());
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "{\"url\":\"https://github.com/parity-bot\"}\n{\"url\":\"https://example.com/a b\"}\n"
        );
    }

    #[test]
    fn without_a_log_nothing_is_written() {
        let mut stderr = Vec::new();
        assert_eq!(run(&["https://github.com".into()], None, &mut stderr), 0);
        assert!(stderr.is_empty());
    }

    #[test]
    fn an_unwritable_log_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut stderr = Vec::new();
        // A directory can't be opened for appending.
        assert_eq!(run(&["https://github.com".into()], Some(dir.path()), &mut stderr), 1);
        assert!(
            String::from_utf8(stderr)
                .unwrap()
                .starts_with("fake-browser: can't write")
        );
    }

    #[test]
    fn opened_by_counts_the_urls_before_each_mark() {
        let text = [
            BrowserLine::Capture {
                capture: "first".into(),
            },
            BrowserLine::Opened {
                url: "https://a".into(),
            },
            BrowserLine::Capture {
                capture: "second".into(),
            },
            BrowserLine::Opened {
                url: "https://b".into(),
            },
            BrowserLine::Capture {
                capture: "third".into(),
            },
        ]
        .iter()
        .map(BrowserLine::to_line)
        .collect::<String>();
        assert_eq!(opened_by(&text, "first"), Vec::<String>::new());
        assert_eq!(opened_by(&text, "second"), ["https://a"]);
        assert_eq!(opened_by(&text, "third"), ["https://a", "https://b"]);
        assert_eq!(opened_by(&text, "missing"), Vec::<String>::new());
        assert!(has_opened(&text));
        assert!(!has_opened(&BrowserLine::Capture { capture: "x".into() }.to_line()));
    }
}
