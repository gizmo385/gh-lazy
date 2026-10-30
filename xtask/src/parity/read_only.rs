//! `env.config_dir_read_only`: the run's config folder made read-only for the app run.
//!
//! This lives outside `env.rs` on purpose: `env.rs` is a [`super::reference::HARNESS_INPUTS`] file, and changing it
//! makes every recorded reference stale. Only Rust-only scenarios use it, and the flag is in the capture key.

use std::path::{Path, PathBuf};

#[cfg(unix)]
use super::Context;
#[cfg(not(unix))]
use super::HarnessError;
use super::Result;

/// The config folder under a run dir, as `env::prepare` makes it.
#[cfg(unix)]
fn config_dir(run_dir: &Path) -> PathBuf {
    run_dir.join("home").join(".config").join("lazy-github")
}

/// The run's config folder, read-only until this drops. Dropping it gives write permission back and then removes the
/// run dir, whose own cleanup couldn't remove the files in a read-only folder.
#[derive(Debug)]
pub struct ReadOnlyConfigDir {
    run_dir: PathBuf,
    dir: PathBuf,
    permissions: std::fs::Permissions,
}

impl ReadOnlyConfigDir {
    /// Takes write permission on `<run_dir>/home/.config/lazy-github` away from everyone (`r-x`). Only on Unix: a
    /// read-only folder on Windows still takes new files.
    #[cfg(unix)]
    pub fn new(run_dir: &Path) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = config_dir(run_dir);
        let permissions = std::fs::metadata(&dir)
            .with_context(|| format!("reading {}'s permissions", dir.display()))?
            .permissions();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
            .with_context(|| format!("making {} read-only", dir.display()))?;
        Ok(Self {
            run_dir: run_dir.to_owned(),
            dir,
            permissions,
        })
    }

    #[cfg(not(unix))]
    pub fn new(_run_dir: &Path) -> Result<Self> {
        Err(HarnessError(
            "`config_dir_read_only` needs Unix: a read-only folder on Windows still takes new files".to_owned(),
        ))
    }
}

impl Drop for ReadOnlyConfigDir {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.dir, self.permissions.clone());
        // The run is over by now; this only removes what was left behind.
        let _ = std::fs::remove_dir_all(&self.run_dir);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[test]
    fn the_folder_is_read_only_until_the_guard_drops_and_the_run_dir_goes_with_it() {
        let run = tempfile::tempdir().unwrap();
        let root = run.path().join("run");
        let dir = config_dir(&root);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), "{}").unwrap();

        let guard = ReadOnlyConfigDir::new(&root).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o555);
        drop(guard);
        assert!(!root.exists());
    }
}
