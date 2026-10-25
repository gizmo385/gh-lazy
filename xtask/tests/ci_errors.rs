//! `cargo xtask parity --ci` annotates the failures that stop a run before any scenario runs.

use std::process::{Command, Output};

fn parity(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("parity")
        .args(args)
        .args(["--ci", "--validate"])
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn an_invalid_scenario_file_is_an_error_on_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("broken.toml");
    std::fs::write(&file, "version = 1\nstpes = []\n").unwrap();
    let output = parity(&[file.as_os_str()]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let out = stdout(&output);
    let line = out
        .lines()
        .find(|l| l.starts_with("::error file="))
        .unwrap_or_else(|| panic!("no ::error annotation in {out:?}"));
    assert!(line.contains("broken.toml,line=2::"), "{line}");
}

#[test]
fn a_harness_error_is_an_error_without_a_file() {
    let output = parity(&["no-such-scenario".as_ref()]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let out = stdout(&output);
    assert!(
        out.lines()
            .any(|l| l.starts_with("::error::parity: no scenario \"no-such-scenario\"")),
        "{out:?}"
    );
}
