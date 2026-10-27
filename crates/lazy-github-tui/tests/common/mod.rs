//! Helpers shared by the golden tests. Each test file that uses them declares `mod common;`.

use std::path::Path;
use std::process::Command;

/// Runs `generator` (relative to the repository root) with `--out <tempdir>` and `HOME` set to `home`, and fails with
/// a diff unless each of `goldens` is byte for byte the file it wrote. `lazy-github-core/tests/config.rs` has its own
/// copy: two short copies are simpler than a shared test crate.
pub fn assert_goldens_fresh(generator: &str, goldens: &[&str], home: &Path, regenerate: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = tempfile::tempdir().unwrap();
    // `uv run` finds (and syncs) the project's interpreter under the real HOME, so uv's cache and Python installs are
    // the usual ones; only the generator sees `home`.
    let found = Command::new("uv")
        .args(["run", "--frozen", "python", "-c", "import sys; print(sys.executable)"])
        .current_dir(&root)
        .output()
        .expect("uv is on PATH");
    assert!(
        found.status.success(),
        "uv run failed:\n{}",
        String::from_utf8_lossy(&found.stderr)
    );
    let python = String::from_utf8(found.stdout).unwrap();
    let run = Command::new(python.trim())
        .arg(generator)
        .arg("--out")
        .arg(out.path())
        .env("HOME", home)
        .env("USERPROFILE", home)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{generator} failed:\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    for golden in goldens {
        let committed = std::fs::read(root.join(golden)).unwrap();
        let fresh = std::fs::read(out.path().join(Path::new(golden).file_name().unwrap())).unwrap();
        if committed != fresh {
            let (committed, fresh) = (String::from_utf8_lossy(&committed), String::from_utf8_lossy(&fresh));
            let diff = similar::TextDiff::from_lines(&*committed, &*fresh)
                .unified_diff()
                .header(golden, "regenerated")
                .to_string();
            let diff: Vec<&str> = diff.lines().take(40).collect();
            panic!(
                "{golden} is stale; the Python app now writes:\n{}\n\n\
                 Regenerate it from the repository root with:\n    {regenerate}",
                diff.join("\n")
            );
        }
    }
}
