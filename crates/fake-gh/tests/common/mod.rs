//! Shared helpers for fake-gh's integration tests.

#![allow(dead_code)] // Each test binary uses a different subset.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// A directory under the system temp dir, removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("fake-gh-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The repository root.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Writes `contents` at `dir/rel`, creating parent directories.
pub fn write(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Copies the fake-gh binary to `<dir>/bin/gh` (`gh.exe` on Windows), as the harness does, and returns the bin dir.
pub fn install_gh(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let name = format!("gh{}", std::env::consts::EXE_SUFFIX);
    let target = bin.join(name);
    std::fs::copy(env!("CARGO_BIN_EXE_fake-gh"), &target).unwrap();
    // A test thread that forked while the copy's write handle was open holds it until its child execs, and exec'ing
    // the copy meanwhile fails with ETXTBSY. Wait that out once here, so tests never see it.
    for _ in 0..100 {
        match Command::new(&target)
            .arg("--version")
            .env_remove("FAKE_GH_LOG")
            .output()
        {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            other => {
                other.unwrap();
                break;
            }
        }
    }
    bin
}

/// A `gh` command (the copied binary) with fake-gh's env cleared down to what's given.
pub fn gh(bin: &Path, fixtures: &[&Path], log: Option<&Path>) -> Command {
    let mut cmd = Command::new(bin.join(format!("gh{}", std::env::consts::EXE_SUFFIX)));
    for var in ["FAKE_GH_FIXTURES", "FAKE_GH_LOG", "FAKE_GH_AUTH", "FAKE_GH_USER"] {
        cmd.env_remove(var);
    }
    if !fixtures.is_empty() {
        cmd.env("FAKE_GH_FIXTURES", std::env::join_paths(fixtures).unwrap());
    }
    if let Some(log) = log {
        cmd.env("FAKE_GH_LOG", log);
    }
    cmd
}

/// Reads a fake-gh log as JSON values.
pub fn read_log(path: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad log line {l:?}: {e}")))
        .collect()
}
