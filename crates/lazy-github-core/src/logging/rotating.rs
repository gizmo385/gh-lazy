//! A size-rotated log file with the same names and rules as Python's `logging.handlers.RotatingFileHandler`, so
//! both apps share one rotation chain.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Appends to a log file and rotates it by size.
///
/// Rotation matches `RotatingFileHandler` in the Python standard library:
///
/// - Before a record is written, the file rolls over if it is non-empty and `len + record >= max_bytes`. An empty
///   file never rolls over, so a single oversized record still gets written.
/// - Rollover shifts `<path>.N-1` to `<path>.N`, …, `<path>.1` to `<path>.2`, moves `<path>` to `<path>.1` and
///   reopens `<path>` empty. `<path>.1` is always the newest backup.
/// - `max_bytes = 0` never rolls over. `backup_count = 0` reopens the same file at rollover, so it keeps growing.
///
/// Each `write` call is treated as one record, which is how `tracing-subscriber`'s fmt layer writes events.
#[derive(Debug)]
pub struct RotatingFileWriter {
    path: PathBuf,
    max_bytes: u64,
    backup_count: u32,
    /// `None` only between closing the file for a rollover and reopening it, or when reopening failed.
    file: Option<File>,
    len: u64,
}

impl RotatingFileWriter {
    /// Opens `path` for appending, creating it and its parent directories if they're missing. The current size of
    /// an existing file counts towards `max_bytes`.
    pub fn open(path: &Path, max_bytes: u64, backup_count: u32) -> io::Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = open_append(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            path: path.to_owned(),
            max_bytes,
            backup_count,
            file: Some(file),
            len,
        })
    }

    fn should_roll_over(&self, record_len: u64) -> bool {
        self.max_bytes > 0 && self.len > 0 && self.len.saturating_add(record_len) >= self.max_bytes
    }

    fn backup_path(&self, n: u32) -> PathBuf {
        let mut name = OsString::from(self.path.as_os_str());
        name.push(format!(".{n}"));
        PathBuf::from(name)
    }

    fn roll_over(&mut self) -> io::Result<()> {
        if self.backup_count > 0 {
            // Windows can't rename a file that is still open.
            self.file = None;
            for n in (1..self.backup_count).rev() {
                let src = self.backup_path(n);
                if src.exists() {
                    replace(&src, &self.backup_path(n + 1))?;
                }
            }
            if self.path.exists() {
                replace(&self.path, &self.backup_path(1))?;
            }
        }
        self.reopen()
    }

    fn reopen(&mut self) -> io::Result<()> {
        let file = open_append(&self.path)?;
        self.len = file.metadata()?.len();
        self.file = Some(file);
        Ok(())
    }
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let record_len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if self.should_roll_over(record_len) {
            // A failed rollover shouldn't lose the record: keep writing to whichever file can be opened. There is
            // nowhere to report the failure, since logs never go to the terminal.
            let _ = self.roll_over();
        }
        if self.file.is_none() {
            self.reopen()?;
        }
        if let Some(file) = self.file.as_mut() {
            // The whole record goes in one call, so a record is never split across files.
            file.write_all(buf)?;
        }
        self.len = self.len.saturating_add(record_len);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), Write::flush)
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Renames `src` to `dst`, first removing `dst`, as Python does, so this also works on Windows.
fn replace(src: &Path, dst: &Path) -> io::Result<()> {
    match std::fs::remove_file(dst) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::rename(src, dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::test_support::TempDir;

    fn record(n: usize) -> Vec<u8> {
        format!("record {n:03}\n").into_bytes() // 11 bytes
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    fn record_numbers(text: &str) -> Vec<usize> {
        text.lines()
            .map(|l| l.trim_start_matches("record ").parse().unwrap())
            .collect()
    }

    #[test]
    fn rotates_at_the_size_threshold() {
        let tmp = TempDir::new();
        let path = tmp.path().join("log");
        let mut w = RotatingFileWriter::open(&path, 100, 2).unwrap();
        for n in 0..50 {
            w.write_all(&record(n)).unwrap();
        }
        w.flush().unwrap();

        let mut names: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["log", "log.1", "log.2"]);

        let (base, one, two) = (
            read(&path),
            read(&tmp.path().join("log.1")),
            read(&tmp.path().join("log.2")),
        );
        for text in [&base, &one, &two] {
            assert!(text.len() < 100, "{} bytes", text.len());
        }
        // 100 bytes fit 9 records of 11 bytes before the 10th would reach the limit.
        assert_eq!(record_numbers(&two), (27..36).collect::<Vec<_>>());
        assert_eq!(record_numbers(&one), (36..45).collect::<Vec<_>>());
        assert_eq!(record_numbers(&base), (45..50).collect::<Vec<_>>());
    }

    #[test]
    fn max_bytes_zero_never_rotates() {
        let tmp = TempDir::new();
        let path = tmp.path().join("log");
        let mut w = RotatingFileWriter::open(&path, 0, 2).unwrap();
        for n in 0..50 {
            w.write_all(&record(n)).unwrap();
        }
        assert_eq!(read(&path).len(), 50 * 11);
        assert!(!tmp.path().join("log.1").exists());
    }

    #[test]
    fn backup_count_zero_keeps_appending() {
        let tmp = TempDir::new();
        let path = tmp.path().join("log");
        let mut w = RotatingFileWriter::open(&path, 100, 0).unwrap();
        for n in 0..50 {
            w.write_all(&record(n)).unwrap();
        }
        assert_eq!(record_numbers(&read(&path)), (0..50).collect::<Vec<_>>());
        assert!(!tmp.path().join("log.1").exists());
    }

    #[test]
    fn reopening_counts_the_existing_size() {
        let tmp = TempDir::new();
        let path = tmp.path().join("log");
        std::fs::write(&path, "x".repeat(95)).unwrap();
        let mut w = RotatingFileWriter::open(&path, 100, 1).unwrap();
        w.write_all(&record(0)).unwrap();
        assert_eq!(read(&tmp.path().join("log.1")), "x".repeat(95));
        assert_eq!(read(&path), "record 000\n");
    }

    #[test]
    fn appends_to_an_existing_file() {
        let tmp = TempDir::new();
        let path = tmp.path().join("log");
        std::fs::write(&path, "record 000\n").unwrap();
        let mut w = RotatingFileWriter::open(&path, 0, 1).unwrap();
        w.write_all(&record(1)).unwrap();
        assert_eq!(record_numbers(&read(&path)), [0, 1]);
    }

    #[test]
    fn an_empty_file_never_rotates() {
        let tmp = TempDir::new();
        let path = tmp.path().join("log");
        let mut w = RotatingFileWriter::open(&path, 10, 2).unwrap();
        let big = "y".repeat(50);
        w.write_all(big.as_bytes()).unwrap();
        assert_eq!(read(&path), big);
        assert!(!tmp.path().join("log.1").exists());

        // The next record rotates the oversized one out.
        w.write_all(b"z").unwrap();
        assert_eq!(read(&tmp.path().join("log.1")), big);
        assert_eq!(read(&path), "z");
    }

    #[test]
    fn creates_missing_parent_directories() {
        let tmp = TempDir::new();
        let path = tmp.path().join("a").join("b").join("lazy_github.log");
        let mut w = RotatingFileWriter::open(&path, 100, 2).unwrap();
        w.write_all(b"hi\n").unwrap();
        assert_eq!(read(&path), "hi\n");
    }
}
