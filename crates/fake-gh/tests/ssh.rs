//! fake-ssh against real git: `git ls-remote` and `git push` over `GIT_SSH`, the refusals, and its log lines next to
//! fake-gh's under the shared lock.

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::{TempDir, gh, install_gh, read_log};

/// A temp dir with `bin/` (gh and fake-ssh, installed as the harness does), `remotes/o/n.git` (a bare repo with
/// `main`) and `work/` (a repo on `main` whose `origin` is `git@github.com:o/n.git`).
struct Setup {
    tmp: TempDir,
    bin: PathBuf,
}

impl Setup {
    fn new(tag: &str) -> Self {
        let tmp = TempDir::new(tag);
        let bin = install_gh(tmp.path());
        let setup = Setup { tmp, bin };
        let (bare, work) = (setup.bare(), setup.work());
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        setup.git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        setup.git(&work, &["init", "-q", "-b", "main"]);
        setup.git(&work, &["remote", "add", "origin", "git@github.com:o/n.git"]);
        setup.git(
            &work,
            &[
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "parity",
            ],
        );
        let bare_arg = bare.to_str().unwrap().to_owned();
        setup.git(&work, &["push", "-q", &bare_arg, "HEAD:refs/heads/main"]);
        setup
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().join("remotes")
    }

    fn bare(&self) -> PathBuf {
        self.root().join("o").join("n.git")
    }

    fn work(&self) -> PathBuf {
        self.tmp.path().join("work")
    }

    fn log(&self) -> PathBuf {
        self.tmp.path().join("fake-gh.log")
    }

    fn fake_ssh(&self) -> PathBuf {
        self.bin.join(fake_gh::fake_ssh_file_name())
    }

    /// `git` in `dir` with the transport on fake-ssh and no developer config.
    fn command(&self, dir: &Path, args: &[&str]) -> Command {
        let mut git = Command::new("git");
        git.args(args)
            .current_dir(dir)
            .env("HOME", self.tmp.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_SSH", self.fake_ssh())
            .env("GIT_SSH_VARIANT", "simple")
            .env("FAKE_SSH_ROOT", self.root())
            .env("FAKE_GH_LOG", self.log())
            .env("GIT_AUTHOR_NAME", "parity-bot")
            .env("GIT_COMMITTER_NAME", "parity-bot")
            .env("GIT_AUTHOR_EMAIL", "parity-bot@example.com")
            .env("GIT_COMMITTER_EMAIL", "parity-bot@example.com")
            .env_remove("GIT_SSH_COMMAND");
        git
    }

    #[track_caller]
    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let output = self.command(dir, args).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    /// fake-ssh run directly with `args`, as git would.
    fn fake_ssh_output(&self, args: &[&str]) -> Output {
        Command::new(self.fake_ssh())
            .args(args)
            .env("FAKE_SSH_ROOT", self.root())
            .env("FAKE_GH_LOG", self.log())
            .output()
            .unwrap()
    }
}

#[test]
fn fake_ssh_is_installed_next_to_gh() {
    let setup = Setup::new("ssh-install");
    assert!(setup.fake_ssh().is_file());
}

#[test]
fn upload_pack_serves_ls_remote_from_the_bare_repo() {
    let setup = Setup::new("ssh-upload");
    let work = setup.work();
    let main = setup.git(&work, &["ls-remote", "origin", "main"]);
    assert!(main.ends_with("\trefs/heads/main\n"), "{main:?}");
    assert_eq!(setup.git(&work, &["ls-remote", "origin", "other"]), "");

    let entries = read_log(&setup.log());
    assert_eq!(entries.len(), 2, "{entries:?}");
    let entry = &entries[0];
    assert_eq!(entry["command"], "ssh");
    assert_eq!(entry["service"], "git-upload-pack");
    assert_eq!(entry["path"], "o/n.git");
    assert_eq!(entry["status"], 0);
    assert_eq!(entry["argv"][0], "git@github.com");
    assert_eq!(entry["argv"][1], "git-upload-pack 'o/n.git'");
}

#[test]
fn receive_pack_pushes_into_the_bare_repo_and_sets_the_upstream() {
    let setup = Setup::new("ssh-receive");
    let work = setup.work();
    setup.git(&work, &["push", "-q", "--set-upstream", "origin", "HEAD:feature"]);
    let bare = setup.bare();
    setup.git(&bare, &["rev-parse", "--verify", "--quiet", "refs/heads/feature"]);
    assert_eq!(
        setup.git(&work, &["config", "--get", "branch.main.merge"]),
        "refs/heads/feature\n"
    );

    let entries = read_log(&setup.log());
    let pushes: Vec<_> = entries.iter().filter(|e| e["service"] == "git-receive-pack").collect();
    assert_eq!(pushes.len(), 1, "{entries:?}");
    assert_eq!(pushes[0]["status"], 0);
    assert_eq!(pushes[0]["path"], "o/n.git");
}

#[test]
fn refusals_exit_128_with_the_github_messages() {
    let setup = Setup::new("ssh-refuse");
    for (command, message) in [
        ("git-upload-archive 'o/n.git'", "fake-ssh: unsupported command"),
        ("git-upload-pack '../work'", "ERROR: Repository not found."),
        ("git-upload-pack 'o/../../work'", "ERROR: Repository not found."),
        ("git-receive-pack 'o/missing.git'", "ERROR: Repository not found."),
    ] {
        let output = setup.fake_ssh_output(&["git@github.com", command]);
        assert_eq!(output.status.code(), Some(128), "{command}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with(message), "{command}: {stderr:?}");
    }
    let entries = read_log(&setup.log());
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(|e| e["command"] == "ssh" && e["status"] == 128));

    // Git reports the refusal as a failed ls-remote.
    let output = setup
        .command(&setup.work(), &["ls-remote", "git@github.com:o/missing.git"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Repository not found"));
}

#[test]
fn ssh_and_gh_lines_share_the_log_lock() {
    let setup = Setup::new("ssh-lock");
    let work = setup.work();
    let children: Vec<_> = (0..16)
        .map(|i| {
            if i % 2 == 0 {
                let mut git = setup.command(&work, &["ls-remote", "origin", "main"]);
                git.stdout(Stdio::null()).spawn().unwrap()
            } else {
                gh(&setup.bin, &[], Some(&setup.log()))
                    .arg("--version")
                    .stdout(Stdio::null())
                    .spawn()
                    .unwrap()
            }
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let entries = read_log(&setup.log());
    assert_eq!(entries.len(), 16);
    let mut seqs: Vec<u64> = entries.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=16).collect::<Vec<_>>());
    assert_eq!(entries.iter().filter(|e| e["command"] == "ssh").count(), 8);
}
