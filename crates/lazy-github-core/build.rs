//! Records the rustc that builds this crate, for the debug info's "Rustc Version" line.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-changed=build.rs");
    // Cargo always sets `RUSTC` for build scripts; `rustc` is the fallback for running this some other way.
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| {
            text.lines()
                .next()
                .and_then(|line| line.strip_prefix("rustc "))
                .map(|v| v.trim().to_owned())
        })
        .filter(|version| !version.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=LAZY_GITHUB_RUSTC_VERSION={version}");
}
