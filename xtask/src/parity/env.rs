//! The isolated environment for one app run: a temp `home/`, `bin/` (fake-gh as `gh`, and `fake-ssh`) and `work/`,
//! the layered `config.json`, an optional git repo with an optional local remote under `remotes/`, and a cleared
//! process environment.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use portable_pty::CommandBuilder;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::scenario::{App, Roots, Scenario, Size, Timing};
use super::{Context, HarnessError, Result};

/// Where the binaries and resources a run needs live.
#[derive(Debug, Clone)]
pub struct Paths {
    /// The fake-gh binary, installed as `bin/gh` in every run.
    pub fake_gh: PathBuf,
    /// The Rust app binary (`target/<profile>/lazy-github`).
    pub rust_app: PathBuf,
    /// The Python interpreter that runs `-m lazy_github` (the repo's venv by default).
    pub python: PathBuf,
    pub roots: Roots,
}

/// Everything the runner needs to start one app at one size.
pub struct RunEnv {
    pub command: CommandBuilder,
    pub size: Size,
    pub timing: Timing,
    /// fake-gh's request log, for `wait.log` steps and the status. `None` when the app doesn't run under fake-gh.
    pub log: Option<PathBuf>,
    /// The temp dir holding `home/`, `bin/` and `work/`; removed when the run finishes.
    pub run_dir: Option<TempDir>,
    /// With `--keep-home`: where `home/` and `work/` (and `remotes/`, if any) are copied when the run finishes.
    pub keep_into: Option<PathBuf>,
    /// The bare repo `origin` reaches over fake-ssh (`git_remote_branches`), for `wait.remote_branch`.
    pub remote: Option<LocalRemote>,
}

/// A run's local remote: the bare repo at `<run>/remotes/<owner>/<name>.git` that fake-ssh serves for `origin`.
#[derive(Debug, Clone)]
pub struct LocalRemote {
    /// The bare repo.
    pub git_dir: PathBuf,
    /// The run's `HOME`, so git reads no developer config.
    pub home: PathBuf,
}

impl LocalRemote {
    /// Whether the bare repo has `refs/heads/<branch>`. A git that can't run counts as no.
    pub fn has_branch(&self, branch: &str) -> bool {
        let mut git = git_command(&self.home);
        git.arg("--git-dir")
            .arg(&self.git_dir)
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("refs/heads/{branch}"));
        git.output().is_ok_and(|o| o.status.success())
    }
}

impl RunEnv {
    /// An environment for an arbitrary command, with no fake-gh and no temp dirs. Tests use it to drive small
    /// programs through the runner.
    pub fn for_command(command: CommandBuilder, size: Size, timing: Timing) -> Self {
        Self {
            command,
            size,
            timing,
            log: None,
            run_dir: None,
            keep_into: None,
            remote: None,
        }
    }

    /// Copies `home/`, `work/` and `remotes/` (when the run has one) into the report if asked to, then removes the
    /// temp dir.
    pub fn finish(self) -> Result<()> {
        if let (Some(run), Some(dest)) = (&self.run_dir, &self.keep_into) {
            for dir in ["home", "work", REMOTES_DIR] {
                let from = run.path().join(dir);
                if dir == REMOTES_DIR && !from.is_dir() {
                    continue;
                }
                copy_dir(&from, &dest.join(dir)).with_context(|| format!("copying {dir}/ into {}", dest.display()))?;
            }
        }
        Ok(())
    }
}

/// The run dir's directory of bare repos, `FAKE_SSH_ROOT`.
const REMOTES_DIR: &str = "remotes";

/// The config every run starts from: `parity/configs/<name>.json`, `config_overrides` deep-merged over it, then
/// the fields that keep the Python app off its first-start screen and on the `gh` backend.
pub fn build_config(base: Value, overrides: &Value) -> Value {
    let mut config = base;
    merge(&mut config, overrides);
    merge(
        &mut config,
        &json!({"core": {"first_start": false}, "api": {"client_type": "GITHUB_CLI"}}),
    );
    config
}

/// The scenario's `config.json`: its config file under `roots.configs` with [`build_config`] applied.
pub fn scenario_config(scenario: &Scenario, roots: &Roots) -> Result<Value> {
    let config_file = roots.config_file(&scenario.env.config);
    let base = std::fs::read_to_string(&config_file).with_context(|| format!("reading {}", config_file.display()))?;
    let base: Value = serde_json::from_str(&base)
        .map_err(|e| HarnessError(format!("{} isn't valid JSON: {e}", config_file.display())))?;
    Ok(build_config(base, &scenario.env.config_overrides))
}

/// Deep-merges `over` into `base`: objects merge key by key, anything else replaces.
fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                merge(base.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (base, over) => *base = over.clone(),
    }
}

/// Builds the run's temp dirs and the command for `app`. `out` is the app's report dir, which gets `fake-gh.log`.
pub fn prepare(
    scenario: &Scenario,
    app: App,
    size: Size,
    paths: &Paths,
    out: &Path,
    keep_home: bool,
) -> Result<RunEnv> {
    let run_dir = tempfile::Builder::new()
        .prefix("lazy-github-parity-")
        .tempdir()
        .context("creating the run dir")?;
    let root = run_dir.path();
    let home = root.join("home");
    let work = root.join("work");
    let config_dir = home.join(".config/lazy-github");
    // `tmp/` is the app's TMPDIR on macOS (see `isolated_env`).
    for dir in [&config_dir, &work, &root.join("tmp")] {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    let config = scenario_config(scenario, &paths.roots)?;
    // A `Value` always serializes.
    let config = serde_json::to_string_pretty(&config).expect("JSON value serializes");
    std::fs::write(config_dir.join("config.json"), config).context("writing config.json")?;

    let bin = fake_gh::install(&paths.fake_gh, root)
        .with_context(|| format!("installing {} as gh", paths.fake_gh.display()))?;

    if let Some(remote) = &scenario.env.git_remote {
        let url = scenario.env.git_remote_protocol.url(remote);
        init_git_repo(&work, &home, &url, &scenario.env.git_branch)?;
    }
    let local_remote = match (&scenario.env.git_remote, &scenario.env.git_remote_branches) {
        (Some(remote), Some(branches)) => {
            if !bin.join(fake_gh::fake_ssh_file_name()).is_file() {
                return Err(HarnessError(format!(
                    "{} has no fake-ssh next to it; build it with `cargo build -p fake-gh`",
                    paths.fake_gh.display()
                )));
            }
            let git_dir = remote_git_dir(root, remote);
            init_bare_remote(&work, &home, &git_dir, branches, &scenario.env.git_branch)?;
            Some(LocalRemote {
                git_dir,
                home: home.clone(),
            })
        }
        _ => None,
    };

    // After `git init`, so the parity commit stays empty (its SHA never changes) and seeded `work/` files are
    // untracked; after `config.json`, which a seed can't hold.
    if let Some(seed) = &scenario.env.seed {
        let seed_dir = paths.roots.seed_dir(seed);
        for (tree, dest) in [("home", &home), ("work", &work)] {
            let from = seed_dir.join(tree);
            if from.is_dir() {
                copy_dir(&from, dest).with_context(|| format!("copying seed {seed}'s {tree}/"))?;
            }
        }
    }

    let log = out.join("fake-gh.log");
    let fixtures = std::env::join_paths(scenario.env.fixtures.iter().map(|set| paths.roots.fixtures.join(set)))
        .map_err(|e| HarnessError(format!("fixture set paths can't be joined: {e}")))?;
    let path = std::env::join_paths(
        std::iter::once(bin.clone()).chain(std::env::var_os("PATH").iter().flat_map(std::env::split_paths)),
    )
    .map_err(|e| HarnessError(format!("PATH can't be built: {e}")))?;

    let mut command = match app {
        App::Python => {
            let mut c = CommandBuilder::new(&paths.python);
            c.args(["-m", "lazy_github"]);
            c
        }
        App::Rust => CommandBuilder::new(&paths.rust_app),
    };
    command.env_clear();
    for (key, value) in isolated_env(root, &home, &work, path, fixtures, &log, scenario) {
        command.env(key, value);
    }
    if local_remote.is_some() {
        for (key, value) in ssh_env(root, &bin) {
            command.env(key, value);
        }
    }
    command.cwd(&work);

    Ok(RunEnv {
        command,
        size,
        timing: scenario.timing,
        log: Some(log),
        run_dir: Some(run_dir),
        keep_into: keep_home.then(|| out.to_owned()),
        remote: local_remote,
    })
}

/// `<run>/remotes/<owner>/<name>.git`, where fake-ssh finds `git@github.com:<owner>/<name>.git`.
fn remote_git_dir(root: &Path, remote: &str) -> PathBuf {
    let mut dir = root.join(REMOTES_DIR);
    // `remote` is a validated `owner/name`.
    for segment in remote.split('/') {
        dir.push(segment);
    }
    dir.set_extension("git");
    dir
}

/// What points git's SSH transport at fake-ssh: the `simple` variant (`<host> <command>`, no ssh options) and the
/// root its repo paths resolve under.
fn ssh_env(root: &Path, bin: &Path) -> Vec<(&'static str, OsString)> {
    vec![
        ("GIT_SSH", bin.join(fake_gh::fake_ssh_file_name()).into()),
        ("GIT_SSH_VARIANT", "simple".into()),
        ("FAKE_SSH_ROOT", root.join(REMOTES_DIR).into()),
    ]
}

/// The allow-listed environment. Nothing else from the developer's environment (`GH_TOKEN`, `NO_COLOR`,
/// `TEXTUAL`, ...) reaches the app.
fn isolated_env(
    root: &Path,
    home: &Path,
    work: &Path,
    path: OsString,
    fixtures: OsString,
    log: &Path,
    scenario: &Scenario,
) -> Vec<(&'static str, OsString)> {
    let mut env: Vec<(&'static str, OsString)> = vec![
        ("HOME", home.into()),
        ("XDG_CONFIG_HOME", home.join(".config").into()),
        ("XDG_CACHE_HOME", home.join(".cache").into()),
        ("XDG_DATA_HOME", home.join(".local/share").into()),
        ("XDG_STATE_HOME", home.join(".local/state").into()),
        ("PATH", path),
        ("TERM", "xterm-256color".into()),
        ("COLORTERM", "truecolor".into()),
        ("TZ", "UTC".into()),
        ("LAZY_GITHUB_ORIGINAL_PWD", work.into()),
        ("FAKE_GH_FIXTURES", fixtures),
        ("FAKE_GH_LOG", log.into()),
        ("FAKE_GH_AUTH", scenario.env.gh_auth.env_value().into()),
        ("TEXTUAL_ANIMATIONS", "none".into()),
        ("PYTHONDONTWRITEBYTECODE", "1".into()),
        ("GIT_CONFIG_NOSYSTEM", "1".into()),
    ];
    if cfg!(windows) {
        env.push(("USERPROFILE", home.into()));
    }
    if cfg!(target_os = "linux") {
        env.push(("LANG", "C.UTF-8".into()));
    }
    if cfg!(target_os = "macos") {
        env.push(("LANG", "en_US.UTF-8".into()));
        env.push(("TMPDIR", root.join("tmp").into()));
    }
    env
}

/// `git` with the developer's environment cleared but `PATH`, and `HOME` the run's, so no user or system config
/// applies.
fn git_command(home: &Path) -> Command {
    let mut git = Command::new("git");
    git.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        git.env("PATH", path);
    }
    git.env("HOME", home).env("GIT_CONFIG_NOSYSTEM", "1");
    git
}

/// Runs `git <args>` in `dir`, with a fixed author and date so commits hash the same on every run.
fn run_git(dir: &Path, home: &Path, args: &[&OsStr]) -> Result<()> {
    let shown = || args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" ");
    let mut git = git_command(home);
    git.args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "parity-bot")
        .env("GIT_COMMITTER_NAME", "parity-bot")
        .env("GIT_AUTHOR_EMAIL", "parity-bot@example.com")
        .env("GIT_COMMITTER_EMAIL", "parity-bot@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
    let output = git.output().with_context(|| format!("running git {}", shown()))?;
    if !output.status.success() {
        return Err(HarnessError(format!(
            "git {} failed: {}",
            shown(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

/// Makes `work` a git repo whose `origin` is `url`, with one empty commit whose SHA is the same on every run.
fn init_git_repo(work: &Path, home: &Path, url: &str, branch: &str) -> Result<()> {
    let runs: [&[&str]; 3] = [
        &["init", "-q", "-b", branch],
        &["remote", "add", "origin", url],
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "parity",
        ],
    ];
    for args in runs {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        run_git(work, home, &args)?;
    }
    Ok(())
}

/// Makes the bare repo `git_dir` with the work repo's commit as each of `branches`, the first its `HEAD`
/// (`fallback_head` when there are none). The pushes go by path, so the work branch gets no upstream.
fn init_bare_remote(work: &Path, home: &Path, git_dir: &Path, branches: &[String], fallback_head: &str) -> Result<()> {
    std::fs::create_dir_all(git_dir).with_context(|| format!("creating {}", git_dir.display()))?;
    let head = branches.first().map_or(fallback_head, String::as_str);
    let init = ["init", "-q", "--bare", "-b", head].map(OsStr::new);
    run_git(git_dir, home, &init)?;
    for branch in branches {
        let refspec = format!("HEAD:refs/heads/{branch}");
        run_git(
            work,
            home,
            &[
                OsStr::new("push"),
                OsStr::new("-q"),
                git_dir.as_os_str(),
                OsStr::new(&refspec),
            ],
        )?;
    }
    Ok(())
}

/// Copies a directory tree. Symlinks are skipped: the run dir holds none that matter, and following them could
/// leave the run dir.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::scenario::GitRemoteProtocol;

    #[test]
    fn overrides_are_deep_merged_and_forced_fields_win() {
        let base = json!({
            "core": {"first_start": true, "keep": 1},
            "api": {"client_type": "RAW_HTTP", "base_url": "x"},
            "appearance": {"theme": "textual-dark", "show_command_log": false},
        });
        let overrides = json!({
            "appearance": {"show_command_log": true},
            "bindings": {"overrides": {"main.repos.focus": "9"}},
            "core": {"first_start": true},
            "api": {"client_type": "RAW_HTTP"},
        });
        assert_eq!(
            build_config(base, &overrides),
            json!({
                "core": {"first_start": false, "keep": 1},
                "api": {"client_type": "GITHUB_CLI", "base_url": "x"},
                "appearance": {"theme": "textual-dark", "show_command_log": true},
                "bindings": {"overrides": {"main.repos.focus": "9"}},
            })
        );
    }

    #[test]
    fn a_non_object_override_replaces_the_value() {
        let config = build_config(json!({"a": {"b": 1}, "core": 5}), &json!({"a": [1, 2]}));
        assert_eq!(config["a"], json!([1, 2]));
        // Forcing `core.first_start` replaces a non-object `core` too.
        assert_eq!(config["core"], json!({"first_start": false}));
    }

    /// `git remote get-url origin` in a work dir set up for `protocol`.
    fn origin_for(protocol: GitRemoteProtocol) -> String {
        let dir = tempfile::tempdir().unwrap();
        let (home, work) = (dir.path().join("home"), dir.path().join("work"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        init_git_repo(&work, &home, &protocol.url("o/n"), "main").unwrap();
        let output = Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(&work)
            .env("HOME", &home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn the_origin_url_follows_the_protocol() {
        assert_eq!(origin_for(GitRemoteProtocol::Ssh), "git@github.com:o/n.git");
        assert_eq!(origin_for(GitRemoteProtocol::Https), "https://github.com/o/n.git");
    }

    #[test]
    fn a_seed_is_copied_in_after_the_config_and_the_git_repo() {
        let dir = tempfile::tempdir().unwrap();
        let parity = Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity");
        let seeds = dir.path().join("seeds");
        for (rel, contents) in [
            ("s/home/.config/lazy-github/.cache/repos.json", "[]"),
            ("s/work/pr_template.md", "template"),
        ] {
            let path = seeds.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        let roots = Roots {
            fixtures: parity.join("fixtures"),
            configs: parity.join("configs"),
            seeds,
        };
        let source = "version = 1\n[env]\ngit_remote = \"o/n\"\nseed = \"s\"\n";
        let scenario = Scenario::parse(Path::new("seeded.toml"), source, &roots).unwrap();
        // Any file will do as fake-gh: nothing runs it.
        let fake_gh = dir.path().join("fake-gh");
        std::fs::write(&fake_gh, "").unwrap();
        let paths = Paths {
            fake_gh: fake_gh.clone(),
            rust_app: fake_gh.clone(),
            python: fake_gh,
            roots,
        };
        let out = dir.path().join("out");
        let run = prepare(&scenario, App::Rust, Size { cols: 80, rows: 24 }, &paths, &out, false).unwrap();
        let root = run.run_dir.as_ref().unwrap().path();
        let (home, work) = (root.join("home"), root.join("work"));
        assert_eq!(
            std::fs::read_to_string(home.join(".config/lazy-github/.cache/repos.json")).unwrap(),
            "[]"
        );
        assert!(home.join(".config/lazy-github/config.json").is_file());
        assert_eq!(
            std::fs::read_to_string(work.join("pr_template.md")).unwrap(),
            "template"
        );

        let git = |args: &[&str]| {
            let output = Command::new("git")
                .args(args)
                .current_dir(&work)
                .env("HOME", &home)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout).unwrap()
        };
        // The parity commit is empty, and the seeded file is untracked.
        assert_eq!(git(&["ls-tree", "-r", "--name-only", "HEAD"]), "");
        assert_eq!(git(&["status", "--porcelain"]), "?? pr_template.md\n");
        run.finish().unwrap();
    }

    #[test]
    fn the_default_config_is_what_the_spec_says() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity/configs/default.json");
        let config: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(
            config,
            json!({"core": {"first_start": false}, "api": {"client_type": "GITHUB_CLI"}})
        );
    }
}
