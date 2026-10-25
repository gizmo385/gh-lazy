//! The real Python gh client code against fake-gh, with no TUI. Needs `uv`, so it's ignored by default:
//! `cargo test -p fake-gh --test python_client -- --ignored`.

mod common;

use std::process::Command;

const SNIPPET: &str = r"
import asyncio
from lazy_github.lib.github.backends.cli import build_command, run_gh_cli_command

query = {'type': 'all', 'direction': 'asc', 'sort': 'full_name', 'page': 1, 'per_page': 50}
response = asyncio.run(run_gh_cli_command(build_command('/user/repos', query_params=query)))
assert response.is_success(), (response.return_code, response.http_status, response.stderr)
repos = response.json()
assert len(repos) == 3, repos
print(' '.join(r['full_name'] for r in repos))
";

#[test]
#[ignore = "needs uv and the Python environment"]
fn python_client_reads_fake_gh_output() {
    let tmp = common::TempDir::new("python-client");
    let bin = common::install_gh(tmp.path());
    let root = common::repo_root();
    let mut path = vec![bin.clone()];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));

    let out = Command::new("uv")
        .args(["run", "--frozen", "python", "-c", SNIPPET])
        .current_dir(&root)
        .env("PATH", std::env::join_paths(path).unwrap())
        .env("FAKE_GH_FIXTURES", root.join("parity/fixtures/base"))
        .env("FAKE_GH_LOG", tmp.path().join("fake-gh.log"))
        .env_remove("FAKE_GH_AUTH")
        .env_remove("FAKE_GH_USER")
        .output()
        .expect("failed to run uv");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        stdout.trim(),
        "parity-bot/dotfiles parity-bot/hello-world parity-bot/lazy-demo"
    );

    let log = common::read_log(&tmp.path().join("fake-gh.log"));
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["miss"], false);
}
