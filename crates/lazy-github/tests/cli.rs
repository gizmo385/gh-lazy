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
const IMPLEMENTED: &[&str] = &["version", "dump-config"];

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
        let (expected, actual) = (expect(field), text(actual));
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
