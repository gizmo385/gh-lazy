//! File-system failures for tests: a write that runs out of space part-way, and (on Unix) folders and files the
//! process may not write or read.

use std::io::{self, Write};

/// The error a full disk gives (`ENOSPC` on Unix).
pub(crate) fn disk_full() -> io::Error {
    io::Error::new(io::ErrorKind::StorageFull, "No space left on device")
}

/// A `write_all` for [`crate::atomic_write::write_with`] that writes the first `n` bytes and then fails as a full
/// disk does, leaving a truncated file behind.
pub(crate) fn disk_full_after(n: usize) -> impl FnOnce(&mut dyn Write, &[u8]) -> io::Result<()> {
    move |file, bytes| {
        file.write_all(&bytes[..n.min(bytes.len())])?;
        Err(disk_full())
    }
}

/// A writer whose every write fails as a full disk does.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FullDisk;

impl Write for FullDisk {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(disk_full())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) use unix::{read_only, unreadable};

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    /// Puts a path's permissions back on drop, so the temp dir holding it can be removed.
    pub(crate) struct Restore {
        path: PathBuf,
        mode: u32,
    }

    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(self.mode));
        }
    }

    fn chmod(path: &Path, mode: u32) -> Restore {
        let old = std::fs::metadata(path).unwrap().permissions().mode();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        Restore {
            path: path.to_owned(),
            mode: old,
        }
    }

    /// Makes the folder `dir` read-only (`r-x`) until the guard drops. `None` when that doesn't stop this process
    /// writing in it, as for root (CI containers may run as root); the test should skip then.
    pub(crate) fn read_only(dir: &Path) -> Option<Restore> {
        let guard = chmod(dir, 0o555);
        let probe = dir.join(".write-probe");
        if std::fs::write(&probe, "").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return None;
        }
        Some(guard)
    }

    /// Makes the file `path` unreadable (`---`) until the guard drops. `None` when this process can read it anyway,
    /// as root can.
    pub(crate) fn unreadable(path: &Path) -> Option<Restore> {
        let guard = chmod(path, 0o000);
        if std::fs::read(path).is_ok() {
            return None;
        }
        Some(guard)
    }
}
