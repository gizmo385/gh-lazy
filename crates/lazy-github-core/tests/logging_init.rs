//! `logging::init` installs a process-wide subscriber, so it gets its own test binary.

use std::sync::Arc;

use jiff::civil::date;
use lazy_github_core::logging::{self, LoggingOptions};

#[test]
fn init_writes_the_file_and_feeds_the_command_log() {
    let dir = std::env::temp_dir().join(format!("lazy-github-logging-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("missing").join("parents").join("lazy_github.log");

    let mut handle = logging::init(LoggingOptions {
        path: path.clone(),
        max_bytes: 5_000_000,
        backup_count: 5,
        clock: Arc::new(|| date(2026, 10, 25).at(14, 3, 7, 0)),
    });
    assert_eq!(handle.file_warning(), None);
    tracing::debug!(target: "lazy_github_core::gh", "Running Github CLI command: gh api -i -X GET /user");
    tracing::info!(target: "lazy_github_tui", "Selected repo foo/bar");

    // Logged before anything read the command log, and still there.
    let line = handle.command_log.try_recv().unwrap();
    assert_eq!(line.to_string(), "INFO 2026-10-25 14:03:07 - Selected repo foo/bar");
    assert!(handle.command_log.try_recv().is_err());

    drop(handle); // Flushes the file.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "DEBUG 2026-10-25 14:03:07 - Running Github CLI command: gh api -i -X GET /user\n\
         INFO 2026-10-25 14:03:07 - Selected repo foo/bar\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
