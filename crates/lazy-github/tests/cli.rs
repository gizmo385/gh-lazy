//! The subcommands other than `run`, against the Python CLI's golden (`parity/goldens/cli.toml`) and the cases where
//! Rust differs on purpose.
//!
//! These point `HOME` at a temp dir, which Windows doesn't use for its config folder, so they only run elsewhere.
#![cfg(not(windows))]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use assert_cmd::Command;
use serde_json::Value;

const GOLDEN: &str = "tests/fixtures/cli/python-cli.json";

/// The subcommands whose golden cases run. A case runs when its first argument is listed here; the issues that add a
/// subcommand add it here.
const IMPLEMENTED: &[&str] = &["version", "dump-config", "clear-config", "clear-cache", "debug"];

struct TempHome(PathBuf);

impl TempHome {
    fn new(name: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lazy-github-cli-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Resolved, as the generator's home is, so the paths the binary prints match the substituted golden.
        Self(dir.canonicalize().unwrap())
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }

    fn config_dir(&self) -> PathBuf {
        self.0.join(".config").join("lazy-github")
    }

    fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.json")
    }

    fn write_config(&self, text: &str) {
        std::fs::create_dir_all(self.config_dir()).unwrap();
        std::fs::write(self.config_file(), text).unwrap();
    }

    /// A command in this home with a cleared environment, as the generator runs Python. Its stdin is a pipe that
    /// closes once `write_stdin`'s text (if any) is written, so a read sees end-of-file as from a closed stdin.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::cargo_bin("lazy-github").unwrap();
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.0)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("LANG", "C.UTF-8")
            .env("TERM", "xterm-256color")
            .current_dir(&self.0);
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Every path under the home, relative to it.
    fn contents(&self) -> Vec<PathBuf> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                out.push(path.strip_prefix(root).unwrap().to_owned());
                if path.is_dir() {
                    walk(&path, root, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &self.0, &mut out);
        out.sort();
        out
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn paths(list: &[&str]) -> Vec<PathBuf> {
    list.iter().map(PathBuf::from).collect()
}

fn golden() -> Value {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN)).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// The cache directory the generator fills: the configured one, or `.cache` in the config folder.
fn cache_dir(home: &TempHome, config: &Value) -> PathBuf {
    match config.pointer("/cache/cache_directory").and_then(Value::as_str) {
        Some(dir) => PathBuf::from(dir.replace("<HOME>", home.path())),
        None => home.config_dir().join(".cache"),
    }
}

/// `debug`'s list lines after the LazyGithub version: Rust lists these where Python lists its libraries (decision
/// `debug-info-versions`).
const DEBUG_LIST: [&str; 4] = ["- Rustc Version: ", "- Gh Version: ", "- OS: ", "- Terminal: "];

/// The text from the `# LazyGithub Config` line on, which `debug` prints as Python does.
fn from_config_heading(stdout: &str) -> String {
    stdout
        .find("# LazyGithub Config\n")
        .map_or(stdout, |start| &stdout[start..])
        .to_owned()
}

/// How `debug`'s "Library Versions" section differs from Rust's list: the heading, the LazyGithub version, then one
/// line for each of [`DEBUG_LIST`] with a value.
fn debug_versions_problems(name: &str, stdout: &str) -> Vec<String> {
    let head = concat!(
        "# Library Versions\n\n- LazyGithub Version: ",
        env!("CARGO_PKG_VERSION"),
        "\n"
    );
    let Some(rest) = stdout.strip_prefix(head) else {
        return vec![format!("{name}: stdout doesn't start with {head:?}:\n{stdout}")];
    };
    let list: Vec<&str> = rest.split("\n\n").next().unwrap_or_default().lines().collect();
    let shaped = list.len() == DEBUG_LIST.len()
        && list
            .iter()
            .zip(DEBUG_LIST)
            .all(|(line, prefix)| line.strip_prefix(prefix).is_some_and(|value| !value.is_empty()));
    if shaped {
        Vec::new()
    } else {
        vec![format!(
            "{name}: the list after the LazyGithub version isn't {DEBUG_LIST:?}:\n{stdout}"
        )]
    }
}

/// Runs one golden case and describes every way the result differs from Python's.
fn check_case(case: &Value) -> Vec<String> {
    let name = case["name"].as_str().unwrap();
    let home = TempHome::new(name);
    let config = &case["config"];
    if !config.is_null() {
        home.write_config(&config.to_string().replace("<HOME>", home.path()));
    }
    let cache = cache_dir(&home, config);
    for file in case["cache_files"].as_array().unwrap() {
        let file = cache.join(file.as_str().unwrap());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "{}").unwrap();
    }

    let args: Vec<&str> = case["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    let mut command = home.command(&args);
    for (key, value) in case["env"].as_object().unwrap() {
        command.env(key, value.as_str().unwrap());
    }
    if let Some(stdin) = case["stdin"].as_str() {
        command.write_stdin(stdin);
    }
    let output = command.output().unwrap();

    let expect = |field: &str| {
        case[field]
            .as_str()
            .unwrap()
            .replace("<HOME>", home.path())
            .replace("<VERSION>", env!("CARGO_PKG_VERSION"))
    };
    let mut problems = Vec::new();
    for (field, actual) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        let (mut expected, mut actual) = (expect(field), text(actual));
        if field == "stdout" && args[0] == "debug" {
            problems.extend(debug_versions_problems(name, &actual));
            (expected, actual) = (from_config_heading(&expected), from_config_heading(&actual));
        }
        if expected != actual {
            let diff = similar::TextDiff::from_lines(&expected, &actual)
                .unified_diff()
                .header("python", "rust")
                .to_string();
            problems.push(format!("{name}: {field} differs:\n{diff}"));
        }
    }
    let actual = [
        ("exit_code", Value::from(output.status.code())),
        ("config_exists_after", Value::from(home.config_file().exists())),
        ("cache_exists_after", Value::from(cache.exists())),
    ];
    for (field, actual) in actual {
        if case[field] != actual {
            problems.push(format!("{name}: {field} is {actual}, Python's is {}", case[field]));
        }
    }
    problems
}

#[test]
fn implemented_subcommands_match_the_python_golden() {
    let golden = golden();
    let cases: Vec<&Value> = golden["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| IMPLEMENTED.contains(&case["args"][0].as_str().unwrap()))
        .collect();
    for subcommand in IMPLEMENTED {
        assert!(
            cases.iter().any(|case| case["args"][0] == *subcommand),
            "the golden has no `{subcommand}` case"
        );
    }
    let problems: Vec<String> = cases.into_iter().flat_map(check_case).collect();
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn clear_auth_without_a_token_prints_the_gh_pointer_and_touches_nothing() {
    let home = TempHome::new("clear-auth-none");
    let output = home.run(&["clear-auth"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        text(&output.stdout),
        "LazyGithub signs in through the GitHub CLI and stores no credentials of its own. To log out, run: gh auth \
         logout\n"
    );
    assert_eq!(text(&output.stderr), "");
    assert_eq!(home.contents(), Vec::<PathBuf>::new());
}

#[test]
fn clear_auth_deletes_the_python_token() {
    let home = TempHome::new("clear-auth-token");
    let token = home.config_dir().join("auth.text");
    std::fs::create_dir_all(home.config_dir()).unwrap();
    std::fs::write(&token, "ghp_secret").unwrap();
    let output = home.run(&["clear-auth"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        text(&output.stdout),
        format!(
            "Removed the token the Python app stored in {}.\nLazyGithub signs in through the GitHub CLI and stores no \
             credentials of its own. To log out, run: gh auth logout\n",
            token.display()
        )
    );
    assert_eq!(text(&output.stderr), "");
    assert_eq!(home.contents(), paths(&[".config", ".config/lazy-github"]));
}

#[test]
fn clear_auth_reports_a_token_it_cannot_delete() {
    let home = TempHome::new("clear-auth-dir");
    let token = home.config_dir().join("auth.text");
    std::fs::create_dir_all(&token).unwrap();
    let output = home.run(&["clear-auth"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    let prefix = format!("Error: failed to delete {}: ", token.display());
    assert!(stderr.starts_with(&prefix), "{stderr:?}");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert_eq!(text(&output.stdout), "");
    assert!(token.is_dir());
}

#[test]
fn run_rejects_auth_backend_and_ignores_its_variable() {
    let home = TempHome::new("run-auth-backend");
    let output = home.run(&["run", "--auth-backend", "GITHUB_CLI"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(text(&output.stderr).contains("--auth-backend"));
    assert_eq!(home.contents(), Vec::<PathBuf>::new());

    // With the variable set, `run` gets past argument parsing and stops at the terminal check as it always does.
    let output = home
        .command(&["run"])
        .env("AUTH_BACKEND", "GITHUB_CLI")
        .output()
        .unwrap();
    assert_eq!(
        text(&output.stderr),
        "Error: LazyGithub needs a terminal, but stdout is not one\n"
    );
    assert!(!home.config_file().exists());
}

#[test]
fn subcommands_other_than_run_create_nothing() {
    let home = TempHome::new("no-side-effects");
    for args in [&["version"][..], &["dump-config"], &["clear-auth"]] {
        let output = home.run(args);
        assert_eq!(output.status.code(), Some(0), "{args:?}: {}", text(&output.stderr));
        assert_eq!(home.contents(), Vec::<PathBuf>::new(), "{args:?}");
    }

    // Nor with a config folder already there: no log file appears in it.
    home.write_config("{}");
    for args in [&["version"][..], &["dump-config"], &["clear-auth"]] {
        home.run(args);
        assert_eq!(
            home.contents(),
            paths(&[".config", ".config/lazy-github", ".config/lazy-github/config.json"]),
            "{args:?}"
        );
    }
}

#[test]
fn dump_config_with_a_broken_config_prints_the_location_then_the_error() {
    let home = TempHome::new("dump-broken");
    home.write_config(r#"{"issues":{"state_filter":"bogus"}}"#);
    let output = home.run(&["dump-config"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stdout),
        format!(
            "Config file location: {} (exists => True)\n",
            home.config_file().display()
        )
    );
    let stderr = text(&output.stderr);
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
    assert!(stderr.contains("`issues.state_filter`"), "{stderr:?}");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
}

#[test]
fn dump_config_shows_unknown_keys_after_the_known_ones() {
    let home = TempHome::new("dump-unknown");
    home.write_config(r#"{"zzz_new": {"on": true}, "issues": {"later": 1, "state_filter": "open"}, "api": {}}"#);
    let output = home.run(&["dump-config"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.contains(
            "  \"issues\": {\n    \"state_filter\": \"open\",\n    \"owner_filter\": \"all\",\n    \"later\": 1\n  },"
        ),
        "{stdout}"
    );
    // An `api` section without `client_type` stays without it, as saved.
    assert!(
        stdout.ends_with(
            "  \"api\": {\n    \"base_url\": \"https://api.github.com\"\n  },\n  \"zzz_new\": {\n    \"on\": true\n  }\n}\n"
        ),
        "{stdout}"
    );
}

/// The one line a failed delete prints (Py bug B97's recommendation), with nothing on stdout after the prompt.
fn assert_failed_delete(output: &Output, path: &Path, prompt: &str) {
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    let prefix = format!("Error: failed to delete {}: ", path.display());
    assert!(stderr.starts_with(&prefix), "{stderr:?}");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert_eq!(text(&output.stdout), format!("{prompt} [y/N]: "));
}

#[test]
fn clear_config_deletes_a_broken_config() {
    let home = TempHome::new("clear-config-broken");
    home.write_config(r#"{"issues":{"state_filter":"bogus"}"#);
    let output = home.command(&["clear-config"]).write_stdin("y\n").output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "Confirm deletion your LazyGithub settings [y/N]: Your settings have been cleared\n"
    );
    assert_eq!(home.contents(), paths(&[".config", ".config/lazy-github"]));
}

#[test]
fn clear_cache_on_a_broken_config_is_one_error_line_and_no_prompt() {
    let home = TempHome::new("clear-cache-broken");
    home.write_config(r#"{"issues":{"state_filter":"bogus"}}"#);
    let cache = home.config_dir().join(".cache");
    std::fs::create_dir_all(&cache).unwrap();
    let output = home.command(&["clear-cache"]).write_stdin("y\n").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "");
    let stderr = text(&output.stderr);
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
    assert!(stderr.contains("`issues.state_filter`"), "{stderr:?}");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
    assert!(cache.is_dir());
}

#[test]
fn clear_config_reports_a_config_it_cannot_delete() {
    let home = TempHome::new("clear-config-dir");
    std::fs::create_dir_all(home.config_file()).unwrap();
    let output = home.command(&["clear-config"]).write_stdin("y\n").output().unwrap();
    assert_failed_delete(
        &output,
        &home.config_file(),
        "Confirm deletion your LazyGithub settings",
    );
    assert!(home.config_file().is_dir());
}

#[test]
fn clear_cache_reports_a_cache_directory_that_is_a_file() {
    let home = TempHome::new("clear-cache-file");
    let cache = home.config_dir().join(".cache");
    std::fs::create_dir_all(home.config_dir()).unwrap();
    std::fs::write(&cache, "not a folder").unwrap();
    let output = home.command(&["clear-cache"]).write_stdin("y\n").output().unwrap();
    let prompt = format!("Confirm deletion of everything in {}", cache.display());
    assert_failed_delete(&output, &cache, &prompt);
    assert!(cache.is_file());
}

#[test]
fn clear_cache_refuses_a_symlinked_cache_directory() {
    let home = TempHome::new("clear-cache-symlink");
    let target = home.0.join("real-cache");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("a.json"), "{}").unwrap();
    let link = home.0.join("linked-cache");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    home.write_config(&format!(r#"{{"cache":{{"cache_directory":"{}"}}}}"#, link.display()));
    let output = home.command(&["clear-cache"]).write_stdin("y\n").output().unwrap();
    let prompt = format!("Confirm deletion of everything in {}", link.display());
    assert_failed_delete(&output, &link, &prompt);
    assert!(link.is_symlink());
    assert!(target.join("a.json").is_file());
}

#[test]
fn clear_config_and_clear_cache_create_no_log_file() {
    let home = TempHome::new("clear-no-log");
    for args in [&["clear-cache", "--no-confirm"][..], &["clear-config", "--no-confirm"]] {
        let output = home.run(args);
        assert_eq!(output.status.code(), Some(0), "{args:?}: {}", text(&output.stderr));
        assert_eq!(home.contents(), Vec::<PathBuf>::new(), "{args:?}");
    }

    // Answered no, with a config folder already there, they leave it as it was.
    home.write_config("{}");
    for args in [&["clear-cache"][..], &["clear-config"]] {
        let output = home.command(args).write_stdin("n\n").output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {}", text(&output.stderr));
        assert_eq!(
            home.contents(),
            paths(&[".config", ".config/lazy-github", ".config/lazy-github/config.json"]),
            "{args:?}"
        );
    }
}

/// `ctrl+c` at the prompt: Click prints `Aborted!` on stderr and exits 1.
#[test]
fn ctrl_c_at_the_prompt_aborts() {
    use std::io::Read as _;
    use std::process::Stdio;

    let home = TempHome::new("clear-ctrl-c");
    home.write_config("{}");
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("lazy-github"))
        .arg("clear-config")
        .env_clear()
        .env("HOME", &home.0)
        .env("LANG", "C.UTF-8")
        .current_dir(&home.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Held open, so the prompt waits for an answer.
    let stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let prompt = "Confirm deletion your LazyGithub settings [y/N]: ";
    let mut seen = Vec::new();
    let mut buf = [0; 256];
    while !seen.ends_with(prompt.as_bytes()) {
        let n = stdout.read(&mut buf).unwrap();
        assert_ne!(n, 0, "stdout closed before the prompt: {:?}", text(&seen));
        seen.extend_from_slice(&buf[..n]);
    }
    let kill = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
    let output = child.wait_with_output().unwrap();
    drop(stdin);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stderr), "Aborted!\n");
    assert_eq!(text(&seen), prompt);
    assert!(home.config_file().is_file());
}

#[test]
fn bug_report_template_matches_pythons() {
    assert_eq!(
        lazy_github_core::debug::bug_report_template("{debug_info}"),
        golden()["bug_report_template"].as_str().unwrap()
    );
}

/// A directory holding only a `gh` that prints what fake-gh's `gh --version` does, for `PATH`.
///
/// fake-gh itself is a binary of another package, which this package's tests can't name, so this stands in for it.
fn fake_gh_bin() -> TempHome {
    use std::os::unix::fs::PermissionsExt as _;

    let bin = TempHome::new("fake-gh-bin");
    let gh = bin.0.join("gh");
    std::fs::write(
        &gh,
        "#!/bin/sh\nprintf 'gh version 2.89.0 (fake-gh)\\nhttps://github.com/cli/cli/releases/tag/v2.89.0\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Another test's fork can briefly hold the write handle; running the script fails with `ETXTBSY` until it lets go.
    let start = std::time::Instant::now();
    while let Err(e) = std::process::Command::new(&gh).output() {
        assert!(
            e.kind() == std::io::ErrorKind::ExecutableFileBusy && start.elapsed().as_secs() < 5,
            "{e}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    bin
}

#[test]
fn debug_lists_the_gh_version_from_path() {
    let home = TempHome::new("debug-fake-gh");
    let bin = fake_gh_bin();
    let output = home.command(&["debug"]).env("PATH", &bin.0).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.contains("\n- Gh Version: 2.89.0 (fake-gh)\n"), "{stdout}");
    assert!(stdout.contains("\n- Terminal: xterm-256color\n"), "{stdout}");
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn debug_without_gh_says_unknown() {
    let home = TempHome::new("debug-no-gh");
    let empty = TempHome::new("debug-empty-path");
    let output = home.command(&["debug"]).env("PATH", &empty.0).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(stdout.contains("\n- Gh Version: unknown\n"), "{stdout}");
    assert_eq!(text(&output.stderr), "");
}

#[test]
fn debug_with_a_broken_config_is_one_error_line() {
    let home = TempHome::new("debug-broken");
    home.write_config(r#"{"issues":{"state_filter":"bogus"}}"#);
    let output = home.run(&["debug"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "");
    let stderr = text(&output.stderr);
    assert!(stderr.starts_with("Error: "), "{stderr:?}");
    assert!(stderr.contains("`issues.state_filter`"), "{stderr:?}");
    assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
}

#[test]
fn debug_creates_nothing() {
    let home = TempHome::new("debug-no-side-effects");
    let output = home.run(&["debug"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(home.contents(), Vec::<PathBuf>::new());

    // No log file appears in a config folder that's already there.
    home.write_config("{}");
    let output = home.run(&["debug"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    assert_eq!(
        home.contents(),
        paths(&[".config", ".config/lazy-github", ".config/lazy-github/config.json"])
    );
}

#[test]
fn debug_shows_unknown_keys() {
    let home = TempHome::new("debug-unknown");
    home.write_config(r#"{"zzz_new": {"on": true}}"#);
    let output = home.run(&["debug"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let stdout = text(&output.stdout);
    assert!(
        stdout.ends_with("    \"zzz_new\": {\n        \"on\": true\n    }\n}\n```\n"),
        "{stdout}"
    );
}

/// Regenerates `python-cli.json` with the Python app and checks the committed copy matches.
#[test]
#[ignore = "needs uv and the Python app"]
fn cli_goldens_are_fresh() {
    let generator = "crates/lazy-github/tests/fixtures/cli/generate.py";
    let golden = "crates/lazy-github/tests/fixtures/cli/python-cli.json";
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = TempHome::new("fresh");
    // `uv run` finds (and syncs) the project's interpreter, so uv's cache and Python installs are the usual ones.
    let found = std::process::Command::new("uv")
        .args(["run", "--frozen", "python", "-c", "import sys; print(sys.executable)"])
        .current_dir(&root)
        .output()
        .expect("uv is on PATH");
    assert!(found.status.success(), "uv run failed:\n{}", text(&found.stderr));
    let python = String::from_utf8(found.stdout).unwrap();
    let run = std::process::Command::new(python.trim())
        .arg(generator)
        .arg("--out")
        .arg(&out.0)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{generator} failed:\n{}{}",
        text(&run.stdout),
        text(&run.stderr)
    );
    let committed = std::fs::read_to_string(root.join(golden)).unwrap();
    let fresh = std::fs::read_to_string(out.0.join("python-cli.json")).unwrap();
    if committed != fresh {
        let diff = similar::TextDiff::from_lines(&committed, &fresh)
            .unified_diff()
            .header(golden, "regenerated")
            .to_string();
        let diff: Vec<&str> = diff.lines().take(40).collect();
        panic!(
            "{golden} is stale; the Python app now writes:\n{}\n\n\
             Regenerate it from the repository root with:\n    uv run python {generator}",
            diff.join("\n")
        );
    }
}
