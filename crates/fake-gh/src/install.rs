//! Putting fake-gh on a `PATH` as `gh`, and fake-ssh and fake-browser next to it.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// How long [`install`] waits for a copied binary to become executable.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

/// The file name of the fake-ssh binary, next to fake-gh in the build dir and in the installed `bin/`.
pub fn fake_ssh_file_name() -> String {
    format!("fake-ssh{}", std::env::consts::EXE_SUFFIX)
}

/// Installs the fake-gh binary at `source` as `<dir>/bin/gh` (`gh.exe` on Windows) and returns `<dir>/bin`, the
/// directory to put first on `PATH`. When the build dir of `source` also holds fake-ssh and fake-browser (Cargo builds
/// every bin of the crate), they're installed as `<dir>/bin/fake-ssh` and `<dir>/bin/fake-browser`.
///
/// It hard-links when it can. Otherwise (e.g. `source` is on another filesystem) it copies, then waits until the copy
/// can be executed: on Unix, a thread elsewhere in the process that forks while the copy's write handle is open holds
/// that handle until its child execs, and exec'ing the copy meanwhile fails with `ETXTBSY`. The handle is
/// close-on-exec and the parent's copy is already closed, so once one exec succeeds the race is over for good.
pub fn install(source: &Path, dir: &Path) -> io::Result<PathBuf> {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin)?;
    install_one(
        source,
        &bin.join(format!("gh{}", std::env::consts::EXE_SUFFIX)),
        &["--version"],
    )?;
    let ssh = source.with_file_name(fake_ssh_file_name());
    if ssh.is_file() {
        // With no arguments fake-ssh only prints its usage; with `FAKE_GH_LOG` unset it logs nothing.
        install_one(&ssh, &bin.join(fake_ssh_file_name()), &[])?;
    }
    let browser = source.with_file_name(crate::browser::file_name());
    if browser.is_file() {
        // With `FAKE_BROWSER_LOG` unset it logs nothing.
        install_one(&browser, &bin.join(crate::browser::file_name()), &[])?;
    }
    Ok(bin)
}

fn install_one(source: &Path, target: &Path, probe: &[&str]) -> io::Result<()> {
    match std::fs::remove_file(target) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    if std::fs::hard_link(source, target).is_ok() {
        return Ok(());
    }
    std::fs::copy(source, target)?;
    wait_until_executable(target, probe)
}

fn wait_until_executable(target: &Path, probe: &[&str]) -> io::Result<()> {
    let start = Instant::now();
    loop {
        // fake-gh's `--version` needs no fixtures and, with the log unset, writes nothing.
        match Command::new(target)
            .args(probe)
            .env_remove("FAKE_GH_LOG")
            .env_remove(crate::browser::LOG_ENV)
            .output()
        {
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
