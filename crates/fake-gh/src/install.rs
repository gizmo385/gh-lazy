//! Putting fake-gh on a `PATH` as `gh`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How long [`install`] waits for a copied binary to become executable.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// Installs the fake-gh binary at `source` as `<dir>/bin/gh` (`gh.exe` on Windows) and returns `<dir>/bin`, the
/// directory to put first on `PATH`.
///
/// It hard-links when it can. Otherwise (e.g. `source` is on another filesystem) it copies, then waits until the copy
/// can be executed: on Unix, a thread elsewhere in the process that forks while the copy's write handle is open holds
/// that handle until its child execs, and exec'ing the copy meanwhile fails with `ETXTBSY`. The handle is
/// close-on-exec and the parent's copy is already closed, so once one exec succeeds the race is over for good.
pub fn install(source: &Path, dir: &Path) -> io::Result<PathBuf> {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin)?;
    let target = bin.join(format!("gh{}", std::env::consts::EXE_SUFFIX));
    match std::fs::remove_file(&target) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    if std::fs::hard_link(source, &target).is_ok() {
        return Ok(bin);
    }
    std::fs::copy(source, &target)?;
    wait_until_executable(&target)?;
    Ok(bin)
}

fn wait_until_executable(target: &Path) -> io::Result<()> {
    let start = Instant::now();
    loop {
        // `--version` needs no fixtures and, with the log unset, writes nothing.
        match Command::new(target).arg("--version").env_remove("FAKE_GH_LOG").output() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy => {
                if start.elapsed() > BUSY_TIMEOUT {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("{} still busy (ETXTBSY) after {BUSY_TIMEOUT:?}", target.display()),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
            Ok(_) => return Ok(()),
        }
    }
}
