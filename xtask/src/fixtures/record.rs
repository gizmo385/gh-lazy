//! `cargo xtask record-fixtures`: runs a set's recipe against the real `gh`, scrubs each response and writes it in
//! fake-gh's fixture format.
//!
//! The network sits behind the [`Gh`] trait: [`GhCli`] runs a real `gh`, and tests pass canned output instead.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use fake_gh::fixture::{FORMAT_VERSION, Fixture, FixtureRequest, RequestKey, Response, dir_for, numbered_file_name};
use fake_gh::store::{LoadedFixture, scan_set};
use serde_json::Value;

use super::check;
use super::recipe::{LoggedMiss, Recipe, RecipeEntry, RecipeError, entries_toml, read_log_misses, sort_entries};
use super::scrub::{RawResponse, ScrubbedBody, ScrubbedResponse, scrub};

/// How many `gh` requests run at once.
const WORKERS: usize = 4;

/// Text bodies longer than this go to a `body_file` sibling instead of `body_text`.
const MAX_INLINE_TEXT: usize = 64 * 1024;

/// What one `gh` invocation printed.
#[derive(Debug, Clone, Default)]
pub struct GhOutput {
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// The `gh` the recorder talks to.
pub trait Gh: Sync {
    /// How to name this gh in messages.
    fn describe(&self) -> String;
    /// `gh --version` output.
    fn version(&self) -> io::Result<String>;
    /// Whether `gh auth status` exits 0.
    fn auth_status_ok(&self) -> io::Result<bool>;
    /// `gh api -i -X GET -H "Accept: <media_type>" <endpoint>`. gh exits 1 for status >= 400 but still prints the
    /// response, so the exit code isn't reported; the status line is what counts.
    fn api_get(&self, media_type: &str, endpoint: &str) -> io::Result<GhOutput>;
}

/// A real `gh` binary.
#[derive(Debug, Clone)]
pub struct GhCli {
    pub program: PathBuf,
}

impl Gh for GhCli {
    fn describe(&self) -> String {
        self.program.display().to_string()
    }

    fn version(&self) -> io::Result<String> {
        let out = Command::new(&self.program)
            .arg("--version")
            .stdin(Stdio::null())
            .output()?;
        Ok(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }

    fn auth_status_ok(&self) -> io::Result<bool> {
        let status = Command::new(&self.program)
            .args(["auth", "status"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        Ok(status.success())
    }

    fn api_get(&self, media_type: &str, endpoint: &str) -> io::Result<GhOutput> {
        let out = Command::new(&self.program)
            .args([
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                &format!("Accept: {media_type}"),
                endpoint,
            ])
            .stdin(Stdio::null())
            .output()?;
        Ok(GhOutput {
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// What the run does with the recorded responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Write fixtures and append `--from-log` misses to the recipe.
    Write,
    /// Record, then compare with the committed fixtures without writing anything.
    Check,
    /// Print the planned requests. Runs no `gh` at all.
    DryRun,
}

pub struct Options<'a> {
    /// `parity/fixtures/<set>`.
    pub set_dir: PathBuf,
    pub recipe: PathBuf,
    pub from_logs: Vec<PathBuf>,
    pub mode: Mode,
    /// `YYYY-MM-DD`, for each fixture's `note`.
    pub today: String,
    pub gh: &'a dyn Gh,
}

/// What a run did (or, in [`Mode::DryRun`] and [`Mode::Check`], would do).
#[derive(Debug, Default)]
pub struct Outcome {
    /// Every request, recipe entries first, then `--from-log` additions.
    pub requests: Vec<RecipeEntry>,
    /// `--from-log` GET misses that weren't in the recipe yet.
    pub added_to_recipe: Vec<RecipeEntry>,
    /// `--from-log` GET misses outside the recording scope, with the reason.
    pub refused: Vec<(RequestKey, String)>,
    /// `--from-log` misses that aren't GET, with their request body.
    pub write_by_hand: Vec<(RequestKey, Option<Value>)>,
    /// One entry per recorded fixture, relative to the set directory.
    pub files: Vec<FileChange>,
    /// Findings from the scrub check and `fake-gh lint` after writing.
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    New,
    Changed,
    Unchanged,
    /// A hand-written fixture with the same key was moved to the canonical name (and its content replaced).
    Renamed {
        from: Vec<PathBuf>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error(transparent)]
    Recipe(#[from] RecipeError),
    #[error("{0}")]
    Log(String),
    #[error("the recipe has requests that can't be recorded:\n{}", indent(.0))]
    Refused(Vec<String>),
    #[error("{0}")]
    Guard(String),
    #[error("fix or remove these fixtures before recording over the set:\n{}", indent(.0))]
    Unloadable(Vec<String>),
    #[error("recording stopped and nothing was written:\n{}", indent(.0))]
    Requests(Vec<String>),
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
}

fn indent(lines: &[String]) -> String {
    lines.iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> RecordError + '_ {
    move |source| RecordError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// `GET /path?a=1 (accept raw)`, for messages.
pub fn describe_key(key: &RequestKey) -> String {
    let mut s = format!("{} {}", key.method, endpoint_for(key));
    if !key.accept.is_json() {
        let _ = write!(s, " (accept {})", key.accept);
    }
    s
}

/// The endpoint passed to `gh api`: the path with the canonical query in it, so `-F` typing can't change values.
pub fn endpoint_for(key: &RequestKey) -> String {
    let mut endpoint = encode_path(&key.path);
    if !key.query.is_empty() {
        let mut query = form_urlencoded::Serializer::new(String::new());
        for (k, v) in &key.query {
            for value in v.values() {
                query.append_pair(k, value);
            }
        }
        endpoint.push('?');
        endpoint.push_str(&query.finish());
    }
    endpoint
}

/// Percent-encodes what a decoded path can't carry literally in a URL.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/!$&'()*+,;=:@".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Runs the recorder.
pub fn run(opts: &Options<'_>) -> Result<Outcome, RecordError> {
    let recipe = Recipe::load(&opts.recipe)?;
    let mut outcome = Outcome::default();
    let mut keys: Vec<RequestKey> = Vec::new();

    let mut refused = Vec::new();
    for entry in &recipe.requests {
        let key = entry.key();
        if let Err(reason) = recipe.check_scope(&key) {
            refused.push(format!("{}: {reason}", describe_key(&key)));
        } else if !keys.contains(&key) {
            keys.push(key);
            outcome.requests.push(entry.clone());
        }
    }
    if !refused.is_empty() {
        return Err(RecordError::Refused(refused));
    }

    for log in &opts.from_logs {
        for miss in read_log_misses(log).map_err(RecordError::Log)? {
            match miss {
                LoggedMiss::Get(entry) => {
                    let key = entry.key();
                    if keys.contains(&key) {
                        continue;
                    }
                    if let Err(reason) = recipe.check_scope(&key) {
                        if !outcome.refused.iter().any(|(k, _)| *k == key) {
                            outcome.refused.push((key, reason));
                        }
                        continue;
                    }
                    keys.push(key);
                    outcome.added_to_recipe.push(entry);
                }
                LoggedMiss::WriteByHand { key, body } => {
                    if !outcome.write_by_hand.iter().any(|(k, b)| *k == key && *b == body) {
                        outcome.write_by_hand.push((key, body));
                    }
                }
            }
        }
    }
    sort_entries(&mut outcome.added_to_recipe);
    outcome.requests.extend(outcome.added_to_recipe.iter().cloned());

    if opts.mode == Mode::DryRun {
        return Ok(outcome);
    }

    // A file that doesn't load isn't known to the planner, which could then write over it under the same name.
    let (existing, load_errors) = if opts.set_dir.is_dir() {
        scan_set(&opts.set_dir, 0)
    } else {
        Default::default()
    };
    if !load_errors.is_empty() {
        return Err(RecordError::Unloadable(
            load_errors.iter().map(ToString::to_string).collect(),
        ));
    }

    guard_gh(opts.gh)?;
    let responses = record_all(opts.gh, &outcome.requests)?;

    let plan = plan_files(&opts.set_dir, &outcome.requests, responses, &existing, &opts.today);

    match opts.mode {
        Mode::Check => {
            outcome.files = plan.iter().map(|p| diff_planned(&opts.set_dir, p)).collect();
        }
        Mode::Write => {
            outcome.files = plan.iter().map(|p| diff_planned(&opts.set_dir, p)).collect();
            if !outcome.added_to_recipe.is_empty() {
                append_to_recipe(&opts.recipe, &outcome.added_to_recipe)?;
            }
            write_plan(&opts.set_dir, &plan)?;
            outcome.problems = check::check_set(&opts.set_dir);
            let lint = fake_gh::lint::lint_sets(std::slice::from_ref(&opts.set_dir));
            outcome
                .problems
                .extend(lint.problems.iter().map(|p| format!("fake-gh lint: {p}")));
        }
        Mode::DryRun => unreachable!("returned above"),
    }
    Ok(outcome)
}

fn guard_gh(gh: &dyn Gh) -> Result<(), RecordError> {
    let version = gh
        .version()
        .map_err(|e| RecordError::Guard(format!("can't run {} --version: {e}", gh.describe())))?;
    if version.contains("fake-gh") {
        return Err(RecordError::Guard(format!(
            "{} is fake-gh ({}), not the real gh. Pass --gh with the real gh, or take fake-gh off PATH.",
            gh.describe(),
            version.lines().next().unwrap_or("").trim()
        )));
    }
    let logged_in = gh
        .auth_status_ok()
        .map_err(|e| RecordError::Guard(format!("can't run {} auth status: {e}", gh.describe())))?;
    if !logged_in {
        return Err(RecordError::Guard(format!(
            "`{} auth status` failed. Log in with `gh auth login` first.",
            gh.describe()
        )));
    }
    Ok(())
}

/// Records every entry, [`WORKERS`] at a time. Any failure stops the run before anything is written.
fn record_all(gh: &dyn Gh, entries: &[RecipeEntry]) -> Result<Vec<ScrubbedResponse>, RecordError> {
    let next = AtomicUsize::new(0);
    let mut results: Vec<Option<Result<ScrubbedResponse, String>>> = vec![None; entries.len()];
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..WORKERS.min(entries.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(entry) = entries.get(i) else { break };
                        done.push((i, record_one(gh, entry)));
                    }
                    done
                })
            })
            .collect();
        for worker in workers {
            // A panic in a worker is a bug in this tool; surface it rather than writing a partial set.
            for (i, result) in worker.join().expect("recording worker panicked") {
                results[i] = Some(result);
            }
        }
    });

    let mut responses = Vec::with_capacity(entries.len());
    let mut failures = Vec::new();
    for (entry, result) in entries.iter().zip(results) {
        match result {
            Some(Ok(resp)) => responses.push(resp),
            Some(Err(reason)) => failures.push(format!("{}: {reason}", describe_key(&entry.key()))),
            None => failures.push(format!("{}: not recorded", describe_key(&entry.key()))),
        }
    }
    if failures.is_empty() {
        Ok(responses)
    } else {
        Err(RecordError::Requests(failures))
    }
}

fn record_one(gh: &dyn Gh, entry: &RecipeEntry) -> Result<ScrubbedResponse, String> {
    let key = entry.key();
    let out = gh
        .api_get(key.accept.media_type(), &endpoint_for(&key))
        .map_err(|e| format!("can't run gh: {e}"))?;
    let raw = RawResponse::parse(&out.stdout).map_err(|e| {
        let stderr = out.stderr.trim();
        if stderr.is_empty() {
            e.to_string()
        } else {
            format!("{e}; gh said: {stderr}")
        }
    })?;
    scrub(raw, entry).map_err(|abort| format!("scrub aborted at {abort}"))
}

/// One fixture to write.
#[derive(Debug)]
struct PlannedFile {
    /// Relative to the set directory.
    rel: PathBuf,
    fixture: Fixture,
    /// The `body_file` sibling's path relative to the set directory, and its bytes.
    sibling: Option<(PathBuf, Vec<u8>)>,
    /// Existing files this one replaces: same-key fixtures under other names, and stale `body_file`s.
    replaces: Vec<PathBuf>,
}

impl PlannedFile {
    fn json(&self) -> String {
        // A `Fixture` is plain data; serializing it can't fail.
        let mut text = serde_json::to_string_pretty(&self.fixture).expect("fixture serializes");
        text.push('\n');
        text
    }
}

/// A hand-written fixture that this recorded key replaces: same key, and no `body` or `when` matcher.
fn is_plain_with_key(f: &LoadedFixture, key: &RequestKey) -> bool {
    let request = &f.fixture.request;
    request.body.is_none() && request.when.is_none() && request.key() == *key
}

fn plan_files(
    set_dir: &Path,
    entries: &[RecipeEntry],
    responses: Vec<ScrubbedResponse>,
    existing: &[LoadedFixture],
    today: &str,
) -> Vec<PlannedFile> {
    let mut plan: Vec<PlannedFile> = Vec::with_capacity(entries.len());
    for (entry, resp) in entries.iter().zip(responses) {
        let key = entry.key();
        let dir = dir_for(&key);
        // The canonical name, unless a fixture with another matcher (or another planned file) already has it.
        let rel = (1..=u32::MAX)
            .map(|n| dir.join(numbered_file_name(&key, n)))
            .find(|rel| {
                let abs = set_dir.join(rel);
                let taken_on_disk = existing.iter().any(|f| f.file == abs && !is_plain_with_key(f, &key));
                !taken_on_disk && !plan.iter().any(|p| p.rel == *rel)
            })
            .expect("some numbered name is free");
        let stem = rel
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".json"))
            .unwrap_or("GET")
            .to_owned();

        let mut response = Response {
            status: resp.status,
            headers: resp.headers,
            body: None,
            body_text: None,
            body_file: None,
            delay_ms: 0,
        };
        let mut sibling_bytes = None;
        match resp.body {
            ScrubbedBody::Empty => {}
            ScrubbedBody::Json(value) => response.body = Some(value),
            ScrubbedBody::Text(text) if text.len() <= MAX_INLINE_TEXT => response.body_text = Some(text),
            ScrubbedBody::Text(text) => sibling_bytes = Some((body_ext(entry), text.into_bytes())),
            ScrubbedBody::Binary(bytes) => sibling_bytes = Some(("bin", bytes)),
        }
        let sibling = sibling_bytes.map(|(ext, bytes)| {
            let name = format!("{stem}.body.{ext}");
            response.body_file = Some(name.clone());
            (dir.join(name), bytes)
        });

        let abs = set_dir.join(&rel);
        let new_sibling = sibling.as_ref().map(|(p, _)| set_dir.join(p));
        let mut replaces = Vec::new();
        for f in existing.iter().filter(|f| is_plain_with_key(f, &key)) {
            if f.file != abs {
                replaces.push(f.file.clone());
            }
            if let Some(old) = f.body_file_path()
                && Some(&old) != new_sibling.as_ref()
            {
                replaces.push(old);
            }
        }

        plan.push(PlannedFile {
            rel,
            fixture: Fixture {
                version: FORMAT_VERSION,
                request: FixtureRequest {
                    method: key.method,
                    path: key.path,
                    query: key.query,
                    accept: key.accept,
                    body: None,
                    when: None,
                },
                response,
                note: Some(format!("recorded {today} by cargo xtask record-fixtures")),
            },
            sibling,
            replaces,
        });
    }
    plan
}

fn body_ext(entry: &RecipeEntry) -> &'static str {
    match entry.accept.as_str() {
        "diff" => "diff",
        "patch" => "patch",
        _ => "txt",
    }
}

/// Compares a planned file with what's committed. The `note` (which holds the recording date) is ignored, so a
/// re-recording with unchanged upstream data reports no changes.
fn diff_planned(set_dir: &Path, planned: &PlannedFile) -> FileChange {
    let abs = set_dir.join(&planned.rel);
    let renamed_from: Vec<PathBuf> = planned
        .replaces
        .iter()
        .filter(|p| fake_gh::store::is_fixture_file(p))
        .map(|p| p.strip_prefix(set_dir).unwrap_or(p).to_path_buf())
        .collect();
    let kind = if !renamed_from.is_empty() {
        ChangeKind::Renamed { from: renamed_from }
    } else if !abs.exists() {
        ChangeKind::New
    } else if same_content(set_dir, &abs, planned) {
        ChangeKind::Unchanged
    } else {
        ChangeKind::Changed
    };
    FileChange {
        path: planned.rel.clone(),
        kind,
    }
}

/// Byte comparison of what a write would produce, with the committed `note` kept: key order and formatting count.
fn same_content(set_dir: &Path, abs: &Path, planned: &PlannedFile) -> bool {
    let Ok(text) = std::fs::read_to_string(abs) else {
        return false;
    };
    let Ok(committed) = serde_json::from_str::<Fixture>(&text) else {
        return false;
    };
    let fresh = PlannedFile {
        rel: PathBuf::new(),
        fixture: Fixture {
            note: committed.note,
            ..planned.fixture.clone()
        },
        sibling: None,
        replaces: Vec::new(),
    };
    let sibling_same = planned
        .sibling
        .as_ref()
        .is_none_or(|(rel, bytes)| std::fs::read(set_dir.join(rel)).is_ok_and(|b| b == *bytes));
    fresh.json() == text && sibling_same && planned.replaces.is_empty()
}

fn write_plan(set_dir: &Path, plan: &[PlannedFile]) -> Result<(), RecordError> {
    for path in plan.iter().flat_map(|p| &p.replaces) {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(path)(e)),
            _ => {}
        }
    }
    for planned in plan {
        let abs = set_dir.join(&planned.rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        if let Some((rel, bytes)) = &planned.sibling {
            let path = set_dir.join(rel);
            std::fs::write(&path, bytes).map_err(io_err(&path))?;
        }
        std::fs::write(&abs, planned.json()).map_err(io_err(&abs))?;
    }
    Ok(())
}

/// Appends `entries` to the recipe as `[[request]]` tables, keeping the existing text (and comments) as they are.
fn append_to_recipe(recipe: &Path, entries: &[RecipeEntry]) -> Result<(), RecordError> {
    let mut text = std::fs::read_to_string(recipe).map_err(io_err(recipe))?;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&entries_toml(entries));
    std::fs::write(recipe, text).map_err(io_err(recipe))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use fake_gh::fixture::{Accept, query_from_pairs};
    use serde_json::json;

    use super::*;
    use crate::fixtures::test_support::{TempDir, write};

    /// A `gh` that answers from canned `-i` output, keyed by endpoint.
    struct StubGh {
        version: String,
        logged_in: bool,
        responses: BTreeMap<String, Vec<u8>>,
        calls: Mutex<Vec<(String, String)>>,
    }

    impl StubGh {
        fn new() -> Self {
            StubGh {
                version: "gh version 2.89.0 (2026-01-01)\nhttps://github.com/cli/cli/releases/tag/v2.89.0\n".into(),
                logged_in: true,
                responses: BTreeMap::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn json(mut self, endpoint: &str, status: u16, body: &Value) -> Self {
            let out = format!(
                "HTTP/1.1 {status} OK\nContent-Type: application/json; charset=utf-8\r\nDate: Sun, 25 Oct 2026 04:00:00 \
                 GMT\r\nX-Ratelimit-Remaining: 4999\r\n\r\n{body}"
            );
            self.responses.insert(endpoint.into(), out.into_bytes());
            self
        }

        fn raw(mut self, endpoint: &str, out: &[u8]) -> Self {
            self.responses.insert(endpoint.into(), out.to_vec());
            self
        }
    }

    impl Gh for StubGh {
        fn describe(&self) -> String {
            "stub-gh".into()
        }

        fn version(&self) -> io::Result<String> {
            Ok(self.version.clone())
        }

        fn auth_status_ok(&self) -> io::Result<bool> {
            Ok(self.logged_in)
        }

        fn api_get(&self, media_type: &str, endpoint: &str) -> io::Result<GhOutput> {
            self.calls.lock().unwrap().push((media_type.into(), endpoint.into()));
            Ok(GhOutput {
                stdout: self.responses.get(endpoint).cloned().unwrap_or_default(),
                stderr: "gh: Not stubbed".into(),
            })
        }
    }

    /// fake-gh itself, run in-process through its library as if it were `gh`.
    struct InProcessFakeGh {
        env: fake_gh::app::Env,
    }

    impl InProcessFakeGh {
        fn new(sets: &[PathBuf]) -> Self {
            InProcessFakeGh {
                env: fake_gh::app::Env {
                    fixtures: Some(sets.to_vec()),
                    log: None,
                    auth: "logged-in".into(),
                    user: "parity-bot".into(),
                },
            }
        }

        fn run(&self, args: &[&str]) -> (u8, Vec<u8>) {
            let args: Vec<String> = args.iter().map(ToString::to_string).collect();
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            let code = fake_gh::app::run(
                std::ffi::OsStr::new("gh"),
                &args,
                &self.env,
                &mut fake_gh::app::Stdio {
                    stdin: &mut io::empty(),
                    stdout: &mut stdout,
                    stderr: &mut stderr,
                },
            );
            (code, stdout)
        }
    }

    impl Gh for InProcessFakeGh {
        fn describe(&self) -> String {
            "in-process fake-gh".into()
        }

        fn version(&self) -> io::Result<String> {
            Ok(String::from_utf8_lossy(&self.run(&["--version"]).1).into_owned())
        }

        fn auth_status_ok(&self) -> io::Result<bool> {
            Ok(self.run(&["auth", "status"]).0 == 0)
        }

        fn api_get(&self, media_type: &str, endpoint: &str) -> io::Result<GhOutput> {
            let accept = format!("Accept: {media_type}");
            let (_, stdout) = self.run(&["api", "-i", "-X", "GET", "-H", &accept, endpoint]);
            Ok(GhOutput {
                stdout,
                stderr: String::new(),
            })
        }
    }

    /// Splits fake-gh's `-i` output into the status line and the body.
    fn split_include(out: &[u8]) -> (String, &[u8]) {
        let status_end = out.iter().position(|&b| b == b'\n').unwrap();
        let body_start = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        (
            String::from_utf8_lossy(&out[..status_end]).into_owned(),
            &out[body_start..],
        )
    }

    #[test]
    fn recorded_fixtures_replay_through_fake_gh() {
        let s = setup();
        s.run(&stub(), Mode::Write, &[]).unwrap();
        let fake = InProcessFakeGh::new(std::slice::from_ref(&s.set));

        // The Python app's argv for the first page of issues.
        let (code, out) = fake.run(&[
            "api",
            "-i",
            "-X",
            "GET",
            "-F",
            "state=all",
            "-F",
            "page=1",
            "-F",
            "per_page=30",
            "/repos/o/r/issues",
        ]);
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&out));
        let (status, body) = split_include(&out);
        assert_eq!(status, "HTTP/2.0 200 OK");
        let issues: Value = serde_json::from_slice(body).unwrap();
        assert_eq!(issues.as_array().map(Vec::len), Some(1));

        let (code, out) = fake.run(&["api", "-H", "Accept: application/vnd.github.raw", "/repos/o/r/readme"]);
        assert_eq!(code, 0);
        assert_eq!(String::from_utf8(out).unwrap(), "# r\n\nMail user@example.com\n");
    }

    #[test]
    fn refuses_to_record_from_fake_gh_itself() {
        let s = setup();
        let fake = InProcessFakeGh::new(std::slice::from_ref(&s.set));
        let err = s.run(&fake, Mode::Write, &[]).unwrap_err().to_string();
        assert!(
            err.contains("in-process fake-gh is fake-gh (gh version 2.89.0 (fake-gh))"),
            "{err}"
        );
        assert!(!s.set.join("repos").exists());
    }

    const RECIPE: &str = r#"version = 1
allowed_repos = ["o/r"]
allowed_users = ["someone"]

# Comments survive --from-log appends.
[[request]]
path = "/repos/o/r"

[[request]]
path = "/repos/o/r/issues"
query = { state = "all", page = "1", per_page = "30" }

[[request]]
path = "/repos/o/r/readme"
accept = "raw"
"#;

    fn stub() -> StubGh {
        StubGh::new()
            .json(
                "/repos/o/r",
                200,
                &json!({"full_name": "o/r", "private": false, "owner": {"login": "o", "avatar_url": "https://avatars.githubusercontent.com/u/9?v=4"}}),
            )
            .json(
                "/repos/o/r/issues?page=1&per_page=30&state=all",
                200,
                &json!([{"number": 1, "title": "Bug", "user": {"login": "x", "gravatar_id": "g"}}]),
            )
            .raw(
                "/repos/o/r/readme",
                b"HTTP/2.0 200 OK\nContent-Type: application/vnd.github.raw; charset=utf-8\r\nEtag: \"x\"\r\n\r\n# r\n\nMail me@corp.test\n",
            )
    }

    struct Setup {
        _tmp: TempDir,
        set: PathBuf,
        recipe: PathBuf,
    }

    fn setup() -> Setup {
        let tmp = TempDir::new("record");
        let set = tmp.path().join("base");
        write(&set, "recipe.toml", RECIPE);
        let recipe = set.join("recipe.toml");
        Setup { _tmp: tmp, set, recipe }
    }

    impl Setup {
        fn run(&self, gh: &dyn Gh, mode: Mode, logs: &[PathBuf]) -> Result<Outcome, RecordError> {
            run(&Options {
                set_dir: self.set.clone(),
                recipe: self.recipe.clone(),
                from_logs: logs.to_vec(),
                mode,
                today: "2026-10-25".into(),
                gh,
            })
        }
    }

    fn read_fixture(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn records_scrubs_and_writes_canonical_fixtures() {
        let s = setup();
        let gh = stub();
        let outcome = s.run(&gh, Mode::Write, &[]).unwrap();
        assert_eq!(outcome.problems, Vec::<String>::new());
        assert_eq!(
            outcome.files.iter().map(|f| (&f.path, &f.kind)).collect::<Vec<_>>(),
            [
                (
                    &["repos", "o", "r", "GET.json"].iter().collect::<PathBuf>(),
                    &ChangeKind::New
                ),
                (
                    &["repos", "o", "r", "issues", "GET.page-1_per_page-30_state-all.json"]
                        .iter()
                        .collect(),
                    &ChangeKind::New
                ),
                (
                    &["repos", "o", "r", "readme", "GET.raw.json"].iter().collect(),
                    &ChangeKind::New
                ),
            ]
        );

        let mut calls = gh.calls.lock().unwrap().clone();
        calls.sort();
        assert_eq!(
            calls,
            [
                ("application/vnd.github+json".to_owned(), "/repos/o/r".to_owned()),
                (
                    "application/vnd.github+json".to_owned(),
                    "/repos/o/r/issues?page=1&per_page=30&state=all".to_owned()
                ),
                ("application/vnd.github.raw".to_owned(), "/repos/o/r/readme".to_owned()),
            ]
        );

        let repo = s.set.join("repos/o/r/GET.json");
        let text = std::fs::read_to_string(&repo).unwrap();
        assert_eq!(
            text,
            r#"{
  "version": 1,
  "request": {
    "method": "GET",
    "path": "/repos/o/r"
  },
  "response": {
    "status": 200,
    "headers": {
      "Content-Type": "application/json; charset=utf-8"
    },
    "body": {
      "full_name": "o/r",
      "private": false,
      "owner": {
        "login": "o",
        "avatar_url": "https://avatars.githubusercontent.com/u/0?v=4"
      }
    }
  },
  "note": "recorded 2026-10-25 by cargo xtask record-fixtures"
}
"#
        );
        let issues = read_fixture(&s.set.join("repos/o/r/issues/GET.page-1_per_page-30_state-all.json"));
        assert_eq!(
            issues["request"]["query"],
            json!({"page": "1", "per_page": "30", "state": "all"})
        );
        assert_eq!(issues["response"]["body"][0]["user"]["gravatar_id"], "");
        let readme = read_fixture(&s.set.join("repos/o/r/readme/GET.raw.json"));
        assert_eq!(readme["request"]["accept"], "raw");
        assert_eq!(readme["response"]["body_text"], "# r\n\nMail user@example.com\n");
        assert_eq!(
            readme["response"]["headers"],
            json!({"Content-Type": "application/vnd.github.raw; charset=utf-8"})
        );
    }

    #[test]
    fn per_page_without_page_is_recorded_as_page_one() {
        let s = setup();
        let recipe = RECIPE.replace(
            r#"query = { state = "all", page = "1", per_page = "30" }"#,
            r#"query = { state = "all", per_page = "30" }"#,
        );
        assert_ne!(recipe, RECIPE);
        write(&s.set, "recipe.toml", &recipe);
        let gh = stub();
        let outcome = s.run(&gh, Mode::Write, &[]).unwrap();
        assert_eq!(outcome.problems, Vec::<String>::new());
        assert!(
            outcome.files.iter().any(|f| f.path
                == ["repos", "o", "r", "issues", "GET.page-1_per_page-30_state-all.json"]
                    .iter()
                    .collect::<PathBuf>()),
            "{:?}",
            outcome.files
        );
        assert!(
            gh.calls
                .lock()
                .unwrap()
                .iter()
                .any(|(_, endpoint)| endpoint == "/repos/o/r/issues?page=1&per_page=30&state=all"),
            "{:?}",
            gh.calls.lock().unwrap()
        );
        let issues = read_fixture(&s.set.join("repos/o/r/issues/GET.page-1_per_page-30_state-all.json"));
        assert_eq!(
            issues["request"]["query"],
            json!({"page": "1", "per_page": "30", "state": "all"})
        );
    }

    #[test]
    fn check_right_after_recording_reports_no_changes() {
        let s = setup();
        s.run(&stub(), Mode::Write, &[]).unwrap();
        // A different day: only the note would differ, and it's ignored.
        let outcome = run(&Options {
            set_dir: s.set.clone(),
            recipe: s.recipe.clone(),
            from_logs: vec![],
            mode: Mode::Check,
            today: "2027-01-01".into(),
            gh: &stub(),
        })
        .unwrap();
        assert!(
            outcome.files.iter().all(|f| f.kind == ChangeKind::Unchanged),
            "{:?}",
            outcome.files
        );

        let changed = stub().json("/repos/o/r", 200, &json!({"full_name": "o/r", "stargazers_count": 2}));
        let outcome = s.run(&changed, Mode::Check, &[]).unwrap();
        assert_eq!(outcome.files[0].kind, ChangeKind::Changed);
        assert_eq!(outcome.files[1].kind, ChangeKind::Unchanged);
        // Check mode writes nothing.
        assert!(
            std::fs::read_to_string(s.set.join("repos/o/r/GET.json"))
                .unwrap()
                .contains("\"owner\"")
        );
    }

    #[test]
    fn check_reports_reordered_keys_and_other_formatting_as_changed() {
        let s = setup();
        s.run(&stub(), Mode::Write, &[]).unwrap();
        let path = s.set.join("repos/o/r/GET.json");
        let committed = std::fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&committed).unwrap();

        let mut reordered = value.clone();
        let body = reordered["response"]["body"].as_object_mut().unwrap();
        let owner = body.shift_remove("owner").unwrap();
        body.insert("owner".into(), owner.clone());
        let full_name = body.shift_remove("full_name").unwrap();
        body.insert("full_name".into(), full_name);
        let variants = [
            (
                "reordered keys",
                serde_json::to_string_pretty(&reordered).unwrap() + "\n",
            ),
            ("compact", serde_json::to_string(&value).unwrap()),
            ("no trailing newline", committed.trim_end().to_owned()),
            ("4-space indent", committed.replace("\n  ", "\n    ")),
        ];
        for (what, text) in variants {
            assert_ne!(text, committed, "{what}");
            std::fs::write(&path, &text).unwrap();
            let outcome = s.run(&stub(), Mode::Check, &[]).unwrap();
            assert_eq!(outcome.files[0].kind, ChangeKind::Changed, "{what}");
            assert_eq!(outcome.files[1].kind, ChangeKind::Unchanged, "{what}");
        }
    }

    #[test]
    fn refuses_to_record_over_a_fixture_that_does_not_load() {
        let s = setup();
        write(&s.set, "repos/o/r/GET.json", "{ not json");
        let gh = stub();
        let err = s.run(&gh, Mode::Write, &[]).unwrap_err().to_string();
        assert!(err.contains("fix or remove these fixtures"), "{err}");
        assert!(err.contains("GET.json"), "{err}");
        assert!(gh.calls.lock().unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(s.set.join("repos/o/r/GET.json")).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn dry_run_touches_nothing_and_runs_no_gh() {
        let s = setup();
        let gh = stub();
        let outcome = s.run(&gh, Mode::DryRun, &[]).unwrap();
        assert_eq!(outcome.requests.len(), 3);
        assert!(gh.calls.lock().unwrap().is_empty());
        assert!(!s.set.join("repos").exists());
    }

    #[test]
    fn refuses_fake_gh_and_logged_out_gh() {
        let s = setup();
        let mut gh = stub();
        gh.version = "gh version 2.89.0 (fake-gh)\nhttps://github.com/cli/cli/releases/tag/v2.89.0\n".into();
        let err = s.run(&gh, Mode::Write, &[]).unwrap_err().to_string();
        assert!(err.contains("is fake-gh"), "{err}");
        assert!(gh.calls.lock().unwrap().is_empty());

        let mut gh = stub();
        gh.logged_in = false;
        let err = s.run(&gh, Mode::Write, &[]).unwrap_err().to_string();
        assert!(err.contains("auth status` failed"), "{err}");
        assert!(gh.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn refuses_out_of_scope_recipe_entries_before_any_request() {
        for (extra, want) in [
            (
                "[[request]]\npath = \"/user/repos\"\n",
                "/user/repos reads the signed-in user's private scope",
            ),
            (
                "[[request]]\npath = \"/notifications\"\n",
                "/notifications reads the signed-in user's private scope",
            ),
            (
                "[[request]]\npath = \"/repos/acme/other\"\n",
                "acme/other is not in allowed_repos",
            ),
            (
                "[[request]]\nmethod = \"PUT\"\npath = \"/repos/o/r/pulls/1/merge\"\n",
                "only GET requests are recorded",
            ),
        ] {
            let s = setup();
            write(&s.set, "recipe.toml", &format!("{RECIPE}\n{extra}"));
            let gh = stub();
            let err = s.run(&gh, Mode::Write, &[]).unwrap_err();
            assert!(matches!(err, RecordError::Refused(_)), "{err}");
            assert!(err.to_string().contains(want), "{err}");
            assert!(gh.calls.lock().unwrap().is_empty());
            assert!(!s.set.join("repos").exists());
        }
    }

    #[test]
    fn a_scrub_abort_writes_nothing() {
        let s = setup();
        let token = format!("gh{}_{}", 'o', "b".repeat(40));
        let gh = stub().json("/repos/o/r", 200, &json!({"full_name": "o/r", "secret": token}));
        let err = s.run(&gh, Mode::Write, &[]).unwrap_err().to_string();
        assert!(
            err.contains("GET /repos/o/r: scrub aborted at JSON pointer \"/secret\""),
            "{err}"
        );
        assert!(!s.set.join("repos").exists());

        let gh = stub().raw("/repos/o/r", b"{\"message\":\"Bad credentials\"}");
        let err = s.run(&gh, Mode::Write, &[]).unwrap_err().to_string();
        assert!(err.contains("no HTTP status line"), "{err}");
        assert!(!s.set.join("repos").exists());
    }

    #[test]
    fn from_log_appends_new_get_misses_and_lists_writes() {
        let s = setup();
        let log = s.set.parent().unwrap().join("fake-gh.log");
        let lines = [
            // Already in the recipe.
            json!({"seq": 1, "argv": [], "command": "api", "method": "GET", "path": "/repos/o/r/issues",
                   "query": {"state": "all", "page": "1", "per_page": "30"}, "accept": "json", "miss": true}),
            json!({"seq": 2, "argv": [], "command": "api", "method": "GET", "path": "/repos/o/r/pulls/3",
                   "accept": "diff", "miss": true}),
            json!({"seq": 3, "argv": [], "command": "api", "method": "PUT", "path": "/repos/o/r/pulls/3/merge",
                   "body": {"merge_method": "squash"}, "miss": true}),
            // Served, not a miss.
            json!({"seq": 4, "argv": [], "command": "api", "method": "GET", "path": "/repos/o/r/labels",
                   "fixture": "base/x.json", "status": 200, "miss": false}),
        ];
        let text: String = lines.iter().map(|l| l.to_string() + "\n").collect();
        std::fs::write(&log, text).unwrap();

        let gh = stub().raw(
            "/repos/o/r/pulls/3",
            b"HTTP/2.0 200 OK\nContent-Type: text/plain; charset=utf-8\r\n\r\ndiff --git a/x b/x\n",
        );
        let outcome = s.run(&gh, Mode::Write, std::slice::from_ref(&log)).unwrap();
        let mut want = RecipeEntry::get("/repos/o/r/pulls/3");
        want.accept = Accept::Diff;
        assert_eq!(outcome.added_to_recipe, [want.clone()]);
        assert_eq!(outcome.write_by_hand.len(), 1);
        assert_eq!(outcome.write_by_hand[0].0.method, "PUT");
        assert_eq!(outcome.write_by_hand[0].1, Some(json!({"merge_method": "squash"})));
        assert_eq!(outcome.requests.len(), 4);

        let recipe_text = std::fs::read_to_string(&s.recipe).unwrap();
        assert!(recipe_text.starts_with(RECIPE), "existing text and comments are kept");
        let recipe = Recipe::load(&s.recipe).unwrap();
        assert_eq!(recipe.requests.len(), 4);
        assert_eq!(recipe.requests[3], want);
        assert!(s.set.join("repos/o/r/pulls/3/GET.diff.json").is_file());

        // Re-running with the same log adds nothing more.
        let outcome = s.run(&gh, Mode::Write, &[log]).unwrap();
        assert!(outcome.added_to_recipe.is_empty());
        assert_eq!(Recipe::load(&s.recipe).unwrap().requests.len(), 4);
    }

    #[test]
    fn from_log_refuses_user_scoped_misses_without_appending_them() {
        let s = setup();
        let log = s.set.parent().unwrap().join("fake-gh.log");
        let line = json!({"seq": 1, "argv": [], "command": "api", "method": "GET", "path": "/user/repos",
                          "query": {"page": "2"}, "miss": true});
        std::fs::write(&log, format!("{line}\n")).unwrap();
        let outcome = s.run(&stub(), Mode::DryRun, &[log]).unwrap();
        assert_eq!(outcome.refused.len(), 1);
        assert_eq!(outcome.refused[0].0.query, query_from_pairs([("page", "2")]));
        assert!(outcome.added_to_recipe.is_empty());
    }

    #[test]
    fn hand_written_fixture_with_the_same_key_is_renamed() {
        let s = setup();
        write(
            &s.set,
            "repos/o/r/my-repo.json",
            r#"{"version":1,"request":{"method":"GET","path":"/repos/o/r"},"response":{"status":200,"body_file":"my-repo.body.txt"}}"#,
        );
        write(&s.set, "repos/o/r/my-repo.body.txt", "old");
        // Same key, but a `when` matcher: a different fixture, left alone.
        let when = r#"{"version":1,"request":{"method":"GET","path":"/repos/o/r","when":{"after":[{"method":"PUT","path":"/x"}]}},"response":{"status":200}}"#;
        write(&s.set, "repos/o/r/GET.json", when);
        // No recipe entry: left alone.
        let other = r#"{"version":1,"request":{"method":"GET","path":"/repos/o/r/labels"},"response":{"status":200,"body":[]}}"#;
        write(&s.set, "repos/o/r/labels/GET.json", other);

        let outcome = s.run(&stub(), Mode::Write, &[]).unwrap();
        assert_eq!(outcome.problems, Vec::<String>::new());
        // The canonical name is taken by the `when` fixture, so the recording gets the next numbered one.
        assert_eq!(
            outcome.files[0].path,
            ["repos", "o", "r", "GET.2.json"].iter().collect::<PathBuf>()
        );
        assert_eq!(
            outcome.files[0].kind,
            ChangeKind::Renamed {
                from: vec![["repos", "o", "r", "my-repo.json"].iter().collect()]
            }
        );
        assert!(!s.set.join("repos/o/r/my-repo.json").exists());
        assert!(!s.set.join("repos/o/r/my-repo.body.txt").exists());
        assert!(s.set.join("repos/o/r/GET.2.json").is_file());
        assert_eq!(std::fs::read_to_string(s.set.join("repos/o/r/GET.json")).unwrap(), when);
        assert_eq!(
            std::fs::read_to_string(s.set.join("repos/o/r/labels/GET.json")).unwrap(),
            other
        );
    }

    #[test]
    fn long_text_and_binary_bodies_go_to_body_files() {
        let s = setup();
        let recipe = format!(
            "{RECIPE}\n[[request]]\npath = \"/repos/o/r/actions/runs/1/logs\"\nbinary = true\n\n[[request]]\npath = \
             \"/repos/o/r/pulls/2\"\naccept = \"diff\"\n"
        );
        write(&s.set, "recipe.toml", &recipe);
        let mut zip = b"HTTP/2.0 200 OK\nContent-Type: application/zip\r\n\r\n".to_vec();
        zip.extend_from_slice(&[b'P', b'K', 3, 4, 0xff]);
        let mut diff = b"HTTP/2.0 200 OK\nContent-Type: text/plain; charset=utf-8\r\n\r\n".to_vec();
        let big = "+x\n".repeat(30_000);
        diff.extend_from_slice(big.as_bytes());
        let gh = stub()
            .raw("/repos/o/r/actions/runs/1/logs", &zip)
            .raw("/repos/o/r/pulls/2", &diff);
        let outcome = s.run(&gh, Mode::Write, &[]).unwrap();
        assert_eq!(outcome.problems, Vec::<String>::new());

        let logs = read_fixture(&s.set.join("repos/o/r/actions/runs/1/logs/GET.json"));
        assert_eq!(logs["response"]["body_file"], "GET.body.bin");
        assert_eq!(
            std::fs::read(s.set.join("repos/o/r/actions/runs/1/logs/GET.body.bin")).unwrap(),
            [b'P', b'K', 3, 4, 0xff]
        );
        let pr = read_fixture(&s.set.join("repos/o/r/pulls/2/GET.diff.json"));
        assert_eq!(pr["response"]["body_file"], "GET.diff.body.diff");
        assert_eq!(
            std::fs::read_to_string(s.set.join("repos/o/r/pulls/2/GET.diff.body.diff")).unwrap(),
            big
        );

        let outcome = s.run(&gh, Mode::Check, &[]).unwrap();
        assert!(
            outcome.files.iter().all(|f| f.kind == ChangeKind::Unchanged),
            "{:?}",
            outcome.files
        );
    }

    #[test]
    fn endpoints_carry_the_canonical_query() {
        let key = RequestKey::from_url(
            "GET",
            "/search/issues",
            &[("q".into(), "repo:o/r is:open".into()), ("page".into(), "1".into())],
            Accept::Json,
        )
        .unwrap();
        assert_eq!(endpoint_for(&key), "/search/issues?page=1&q=repo%3Ao%2Fr+is%3Aopen");
        // fake-gh's normalization reads it back as the same key.
        assert_eq!(
            RequestKey::from_url("GET", &endpoint_for(&key), &[], Accept::Json).unwrap(),
            key
        );
        assert_eq!(encode_path("/repos/o/r/contents/a b%"), "/repos/o/r/contents/a%20b%25");
    }
}
