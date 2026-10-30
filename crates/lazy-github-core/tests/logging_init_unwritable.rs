//! `logging::init` with a log file that can't be opened. A separate test binary, since `init` is process-wide. The
//! stderr line itself is checked by the `lazy-github` binary's tests.

use lazy_github_core::logging::{self, LoggingOptions};

#[test]
fn init_without_a_log_file_still_feeds_the_command_log() {
    let dir = std::env::temp_dir().join(format!("lazy-github-logging-unwritable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A regular file where the log's parent directory should be.
    let blocker = dir.join("not-a-dir");
    std::fs::write(&blocker, "").unwrap();

    let mut handle = logging::init(LoggingOptions {
        path: blocker.join("lazy_github.log"),
        max_bytes: 5_000_000,
        backup_count: 5,
        clock: logging::system_clock(),
    });
    let warning = handle.file_warning().unwrap();
    let prefix = format!(
        "Could not open log file {}: ",
        blocker.join("lazy_github.log").display()
    );
    assert!(warning.starts_with(&prefix), "{warning}");
    assert!(warning.ends_with(". Continuing without a log file."), "{warning}");
    tracing::info!(target: "lazy_github_tui", "Selected repo foo/bar");
    let line = handle.command_log.try_recv().unwrap();
    assert_eq!(line.message, "Selected repo foo/bar");
    let _ = std::fs::remove_dir_all(&dir);
}
