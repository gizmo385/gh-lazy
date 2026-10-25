//! The fake-gh process: dispatches a parsed command, consults fixtures and the log, prints the answer.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::argv::{self, ApiRequest, Command};
use crate::fixture::{Accept, KeyError, RequestKey};
use crate::lint;
use crate::log::{LockedLog, LogEntry, RequestLog};
use crate::output::{Rendered, write_api_response};
use crate::store::{FixtureStore, Lookup, Request};

/// Exit code for a successful command.
pub const EXIT_OK: u8 = 0;
/// Exit code for an HTTP error, a miss, or being logged out (what real gh returns for these).
pub const EXIT_FAILED: u8 = 1;
/// Exit code for argv fake-gh doesn't support.
pub const EXIT_USAGE: u8 = 2;
/// Exit code for a fake-gh configuration problem: missing or broken fixtures, ambiguous matches, log I/O.
pub const EXIT_CONFIG: u8 = 3;

const VERSION_TEXT: &str = "gh version 2.89.0 (fake-gh)\nhttps://github.com/cli/cli/releases/tag/v2.89.0\n";
const TOKEN: &str = "fake-gh-token";

/// fake-gh's configuration, read from `FAKE_GH_*` environment variables.
#[derive(Debug, Clone)]
pub struct Env {
    /// `FAKE_GH_FIXTURES`, split with the platform path separator.
    pub fixtures: Option<Vec<PathBuf>>,
    /// `FAKE_GH_LOG`.
    pub log: Option<PathBuf>,
    /// `FAKE_GH_AUTH`: `logged-in` (default) or `logged-out`. Anything else is kept to report it.
    pub auth: String,
    /// `FAKE_GH_USER`, default `parity-bot`.
    pub user: String,
}

impl Env {
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        Env {
            fixtures: var("FAKE_GH_FIXTURES").map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            }),
            log: var("FAKE_GH_LOG").map(PathBuf::from),
            auth: var("FAKE_GH_AUTH").map_or_else(|| "logged-in".into(), |v| v.to_string_lossy().into_owned()),
            user: var("FAKE_GH_USER").map_or_else(|| "parity-bot".into(), |v| v.to_string_lossy().into_owned()),
        }
    }
}

/// The process's standard streams.
pub struct Stdio<'a> {
    pub stdin: &'a mut dyn Read,
    pub stdout: &'a mut dyn Write,
    pub stderr: &'a mut dyn Write,
}

/// Runs fake-gh. `program` is argv[0]: when its file stem is `fake-gh`, `fake-gh lint` is available.
pub fn run(program: &OsStr, args: &[String], env: &Env, io: &mut Stdio<'_>) -> u8 {
    let invoked_as_fake_gh = Path::new(program).file_stem() == Some(OsStr::new("fake-gh"));
    if invoked_as_fake_gh && args.first().map(String::as_str) == Some("lint") {
        return run_lint(&args[1..], io);
    }
    let mut ctx = Ctx { args, env, io };
    match ctx.run() {
        Ok(code) => code,
        Err(Failure {
            code,
            message,
            entry,
            log_broken,
        }) => {
            let _ = writeln!(ctx.io.stderr, "fake-gh: {message}");
            // When the log itself is what failed, logging the failure would only fail (and print) again.
            if !log_broken {
                let mut entry = entry.map(|e| *e).unwrap_or_default();
                entry.argv = args.to_vec();
                entry.error = Some(message);
                // Already failing; a log problem here has nowhere better to go than stderr.
                if let Err(e) = ctx.log_only(entry) {
                    let _ = writeln!(ctx.io.stderr, "fake-gh: {e}");
                }
            }
            code
        }
    }
}

fn run_lint(sets: &[String], io: &mut Stdio<'_>) -> u8 {
    if sets.is_empty() {
        let _ = writeln!(io.stderr, "usage: fake-gh lint <set-dir>...");
        return EXIT_USAGE;
    }
    let dirs: Vec<PathBuf> = sets.iter().map(PathBuf::from).collect();
    let report = lint::lint_sets(&dirs);
    for p in &report.problems {
        let _ = writeln!(io.stderr, "fake-gh lint: {p}");
    }
    if report.problems.is_empty() {
        let _ = writeln!(
            io.stdout,
            "fake-gh lint: {} fixtures in {} sets, no problems",
            report.fixtures,
            dirs.len()
        );
        EXIT_OK
    } else {
        let _ = writeln!(io.stderr, "fake-gh lint: {} problem(s)", report.problems.len());
        EXIT_FAILED
    }
}

struct Failure {
    code: u8,
    message: String,
    /// What's known about the request so far, for the log line.
    entry: Option<Box<LogEntry>>,
    /// The failure is the log file itself, so don't try to log it.
    log_broken: bool,
}

impl Failure {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Failure {
            code,
            message: message.into(),
            entry: None,
            log_broken: false,
        }
    }

    /// The log couldn't be opened or written.
    fn log(message: impl Into<String>) -> Self {
        Failure {
            log_broken: true,
            ..Failure::new(EXIT_CONFIG, message)
        }
    }

    fn with_entry(mut self, entry: &LogEntry) -> Self {
        self.entry = Some(Box::new(entry.clone()));
        self
    }
}

struct Ctx<'a, 'io> {
    args: &'a [String],
    env: &'a Env,
    io: &'a mut Stdio<'io>,
}

/// One served page of a response.
struct Page {
    rendered: Rendered,
    delay_ms: u64,
}

impl Ctx<'_, '_> {
    fn run(&mut self) -> Result<u8, Failure> {
        let command = argv::parse(self.args).map_err(|e| Failure::new(EXIT_USAGE, e.to_string()))?;
        let base = LogEntry {
            argv: self.args.to_vec(),
            command: Some(command.log_name().to_owned()),
            ..LogEntry::default()
        };
        match command {
            Command::Version => {
                self.log_only(base).map_err(Failure::log)?;
                self.out(VERSION_TEXT.as_bytes());
                Ok(EXIT_OK)
            }
            Command::AuthStatus | Command::AuthToken => self.auth(&command, base),
            Command::PrDiff {
                owner,
                repo,
                number,
                patch,
            } => {
                let accept = if patch { Accept::Patch } else { Accept::Diff };
                let path = format!("/repos/{owner}/{repo}/pulls/{number}");
                self.pr_diff(&path, &accept, base)
            }
            Command::Api(req) => self.api(&req, base),
        }
    }

    fn out(&mut self, bytes: &[u8]) {
        // A closed stdout (the app went away) isn't fake-gh's problem to report.
        let _ = self.io.stdout.write_all(bytes);
        let _ = self.io.stdout.flush();
    }

    fn err(&mut self, text: &str) {
        let _ = self.io.stderr.write_all(text.as_bytes());
    }

    fn log_only(&self, entry: LogEntry) -> Result<(), String> {
        match &self.env.log {
            Some(path) => {
                let log = open_log(path)?;
                log.append(entry)
                    .map(|_| ())
                    .map_err(|e| format!("can't write log {}: {e}", path.display()))
            }
            None => Ok(()),
        }
    }

    fn logged_in(&self) -> Result<bool, Failure> {
        match self.env.auth.as_str() {
            "logged-in" => Ok(true),
            "logged-out" => Ok(false),
            other => Err(Failure::new(
                EXIT_CONFIG,
                format!("FAKE_GH_AUTH must be logged-in or logged-out, not {other:?}"),
            )),
        }
    }

    fn auth(&mut self, command: &Command, base: LogEntry) -> Result<u8, Failure> {
        let logged_in = self.logged_in().map_err(|f| f.with_entry(&base))?;
        self.log_only(base).map_err(Failure::log)?;
        match (command, logged_in) {
            (Command::AuthStatus, true) => {
                let text = format!(
                    "github.com\n  ✓ Logged in to github.com account {} (keyring)\n  - Active account: true\n  - Git operations protocol: https\n  - Token: gho_************************************\n",
                    self.env.user
                );
                self.out(text.as_bytes());
                Ok(EXIT_OK)
            }
            (Command::AuthStatus, false) => {
                self.err("You are not logged into any GitHub hosts. To log in, run: gh auth login\n");
                Ok(EXIT_FAILED)
            }
            (_, true) => {
                self.out(format!("{TOKEN}\n").as_bytes());
                Ok(EXIT_OK)
            }
            (_, false) => {
                self.err("no oauth token found for github.com\n");
                Ok(EXIT_FAILED)
            }
        }
    }

    fn store(&self, entry: &LogEntry) -> Result<FixtureStore, Failure> {
        let sets = self
            .env
            .fixtures
            .as_ref()
            .ok_or_else(|| Failure::new(EXIT_CONFIG, "FAKE_GH_FIXTURES is not set").with_entry(entry))?;
        FixtureStore::load(sets).map_err(|e| Failure::new(EXIT_CONFIG, e.to_string()).with_entry(entry))
    }

    fn lock_log(&self) -> Result<Option<LockedLog>, Failure> {
        self.env.log.as_deref().map(open_log).transpose().map_err(Failure::log)
    }

    fn pr_diff(&mut self, path: &str, accept: &Accept, mut entry: LogEntry) -> Result<u8, Failure> {
        let key = RequestKey::from_url("GET", path, &[], accept.clone())
            .map_err(|e| Failure::new(EXIT_USAGE, e.to_string()))?;
        entry.method = Some(key.method.clone());
        entry.path = Some(key.path.clone());
        entry.query = Some(key.query.clone());
        entry.accept = Some(accept.to_string());
        let store = self.store(&entry)?;
        let locked = self.lock_log()?;
        let log = locked.as_ref().map_or_else(RequestLog::disabled, |l| l.log().clone());

        let page = match store.lookup(&Request { key, body: None }, &log) {
            Lookup::Hit(f) => {
                entry.fixture = Some(f.display.clone());
                let rendered = Rendered::from_response(&f.fixture.response, f.body_file_path().as_deref())
                    .map_err(|e| Failure::new(EXIT_CONFIG, format!("{}: {e}", f.display)).with_entry(&entry))?;
                Page {
                    rendered,
                    delay_ms: f.fixture.response.delay_ms,
                }
            }
            Lookup::Miss => {
                entry.miss = true;
                Page {
                    rendered: Rendered::not_found(),
                    delay_ms: 0,
                }
            }
            Lookup::Ambiguous(fs) => return Err(ambiguous(&fs, &entry)),
        };
        entry.status = Some(page.rendered.status);
        if let Some(locked) = locked {
            locked
                .append(entry)
                .map_err(|e| Failure::log(format!("can't write log: {e}")))?;
        }
        sleep_ms(page.delay_ms);
        if page.rendered.status >= 400 {
            self.err(&format!(
                "could not find pull request diff: HTTP {}\n",
                page.rendered.status
            ));
            return Ok(EXIT_FAILED);
        }
        self.out(&page.rendered.body);
        Ok(EXIT_OK)
    }

    fn api(&mut self, req: &ApiRequest, mut entry: LogEntry) -> Result<u8, Failure> {
        let method = req.method();
        let accept = req.accept();
        entry.method = Some(method.clone());
        entry.accept = Some(accept.to_string());
        entry.cache.clone_from(&req.cache);
        entry.cache_control = req.cache_control().map(str::to_owned);

        let (extra_query, body) = self.request_body(req).map_err(|f| f.with_entry(&entry))?;
        entry.body.clone_from(&body);
        let endpoint = if req.is_graphql() {
            "/graphql"
        } else {
            req.endpoint.as_str()
        };
        let first = match RequestKey::from_url(&method, endpoint, &extra_query, accept) {
            Ok(key) => Some(key),
            Err(KeyError::ForeignHost(_)) => None,
            Err(e @ KeyError::Placeholder(_)) => return Err(Failure::new(EXIT_USAGE, e.to_string()).with_entry(&entry)),
        };
        match &first {
            Some(key) => {
                entry.path = Some(key.path.clone());
                entry.query = Some(key.query.clone());
            }
            None => entry.path = Some(req.endpoint.clone()),
        }

        let store = self.store(&entry)?;
        let locked = self.lock_log()?;
        let log = locked.as_ref().map_or_else(RequestLog::disabled, |l| l.log().clone());
        let first = first.map(|key| Request { key, body });
        let pages = fetch_pages(&store, &log, first, req.paginate, &mut entry)?;
        let last_status = pages.last().map_or(404, |p| p.rendered.status);
        entry.status = Some(last_status);
        if let Some(locked) = locked {
            locked
                .append(entry)
                .map_err(|e| Failure::log(format!("can't write log: {e}")))?;
        }

        self.out(&render_pages(&pages, req.include, req.slurp));
        if let Some(line) = pages.last().and_then(|p| p.rendered.error_line()) {
            self.err(&line);
            return Ok(EXIT_FAILED);
        }
        Ok(EXIT_OK)
    }

    /// Builds the extra query pairs and the request body from `-F`/`-f` and `--input`.
    fn request_body(&mut self, req: &ApiRequest) -> Result<QueryAndBody, Failure> {
        let mut stdin_text: Option<String> = None;
        let mut read_stdin = |stdin: &mut dyn Read| -> io::Result<String> {
            if let Some(t) = &stdin_text {
                return Ok(t.clone());
            }
            let mut text = String::new();
            stdin.read_to_string(&mut text)?;
            stdin_text = Some(text.clone());
            Ok(text)
        };
        let stdin = &mut *self.io.stdin;
        let typed = argv::typed_fields(&req.fields, &mut |path| {
            if path == "-" {
                read_stdin(stdin)
            } else {
                std::fs::read_to_string(path)
            }
        })
        .map_err(|e| Failure::new(EXIT_USAGE, format!("can't read field value: {e}")))?;

        let mut body = None;
        if let Some(input) = &req.input {
            let text = if input == "-" {
                read_stdin(stdin)
            } else {
                std::fs::read_to_string(input)
            }
            .map_err(|e| Failure::new(EXIT_USAGE, format!("can't read --input {input}: {e}")))?;
            body = Some(Request::body_value(&text));
        }
        if req.fields_in_query() {
            Ok((argv::query_pairs(&typed), body))
        } else {
            let fields_body = (!typed.is_empty()).then(|| argv::json_body(&typed, req.is_graphql()));
            Ok((Vec::new(), body.or(fields_body)))
        }
    }
}

/// Extra query pairs from fields, and the request body.
type QueryAndBody = (Vec<(String, String)>, Option<Value>);

/// Looks up the first request and, with `paginate`, follows `Link: rel="next"` from page to page. Always returns at
/// least one page; a miss ends the list with GitHub's 404.
fn fetch_pages(
    store: &FixtureStore,
    log: &RequestLog,
    first: Option<Request>,
    paginate: bool,
    entry: &mut LogEntry,
) -> Result<Vec<Page>, Failure> {
    let method = entry.method.clone().unwrap_or_default();
    let mut pages: Vec<Page> = Vec::new();
    let mut next = first;
    loop {
        let Some(request) = next.take() else {
            // A foreign host, or a `Link` to one: GitHub's answer would be a 404 too.
            entry.miss = true;
            entry.fixture = None;
            pages.push(Page::not_found());
            return Ok(pages);
        };
        match store.lookup(&request, log) {
            Lookup::Hit(f) => {
                if pages.is_empty() {
                    entry.fixture = Some(f.display.clone());
                }
                let rendered = Rendered::from_response(&f.fixture.response, f.body_file_path().as_deref())
                    .map_err(|e| Failure::new(EXIT_CONFIG, format!("{}: {e}", f.display)).with_entry(entry))?;
                let link = (paginate && rendered.status < 400)
                    .then(|| rendered.next_link())
                    .flatten();
                pages.push(Page {
                    rendered,
                    delay_ms: f.fixture.response.delay_ms,
                });
                let Some(url) = link else {
                    return Ok(pages);
                };
                next = RequestKey::from_url(&method, &url, &[], request.key.accept.clone())
                    .ok()
                    .map(|key| Request { key, body: None });
            }
            Lookup::Miss => {
                // Log the key that missed, so the recorder (#68) can fill it in, even when it's page 2.
                entry.miss = true;
                entry.fixture = None;
                entry.path = Some(request.key.path.clone());
                entry.query = Some(request.key.query.clone());
                pages.push(Page::not_found());
                return Ok(pages);
            }
            Lookup::Ambiguous(fs) => return Err(ambiguous(&fs, entry)),
        }
    }
}

/// Prints pages back to back, or with `slurp` wrapped in one JSON array. Sleeps for each page's `delay_ms` first.
fn render_pages(pages: &[Page], include: bool, slurp: bool) -> Vec<u8> {
    let mut buf = Vec::new();
    let failed = pages.last().is_some_and(|p| p.rendered.status >= 400);
    if slurp && !failed {
        for p in pages {
            sleep_ms(p.delay_ms);
        }
        if include && let Some(first) = pages.first() {
            // Only the first page's headers: slurped output is one document.
            let header_only = Rendered {
                body: Vec::new(),
                ..first.rendered.clone()
            };
            let _ = write_api_response(&mut buf, &header_only, true);
        }
        buf.push(b'[');
        for (i, p) in pages.iter().enumerate() {
            if i > 0 {
                buf.push(b',');
            }
            buf.extend_from_slice(&p.rendered.body);
        }
        buf.push(b']');
    } else {
        if !include
            && !slurp
            && pages.len() > 1
            && !failed
            && let Some(merged) = merge_array_pages(pages)
        {
            for p in pages {
                sleep_ms(p.delay_ms);
            }
            return merged;
        }
        // A failed slurp shows just the error, as gh stops at the failing page.
        let shown = if slurp {
            &pages[pages.len().saturating_sub(1)..]
        } else {
            pages
        };
        for p in shown {
            sleep_ms(p.delay_ms);
            // Writing to a Vec can't fail.
            let _ = write_api_response(&mut buf, &p.rendered, include);
        }
    }
    buf
}

/// What real gh's `paginatedArrayReader` does for `--paginate` without `-i`/`--slurp`: when every page is a JSON
/// array, `[a,b]` and `[c]` come out as `[a,b,c]`. Works on the bytes, so each element is printed exactly as stored.
/// `None` when some page isn't an array; those print back to back.
fn merge_array_pages(pages: &[Page]) -> Option<Vec<u8>> {
    let mut inners = Vec::with_capacity(pages.len());
    for p in pages {
        let body = p.rendered.body.trim_ascii();
        if !serde_json::from_slice::<Value>(body).is_ok_and(|v| v.is_array()) {
            return None;
        }
        // A valid JSON array starts with `[` and ends with `]`.
        let inner = body[1..body.len() - 1].trim_ascii();
        if !inner.is_empty() {
            inners.push(inner);
        }
    }
    let mut out = vec![b'['];
    for (i, inner) in inners.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(inner);
    }
    out.push(b']');
    Some(out)
}

impl Page {
    fn not_found() -> Self {
        Page {
            rendered: Rendered::not_found(),
            delay_ms: 0,
        }
    }
}

fn ambiguous(fixtures: &[&crate::store::LoadedFixture], entry: &LogEntry) -> Failure {
    let names: Vec<&str> = fixtures.iter().map(|f| f.display.as_str()).collect();
    Failure::new(
        EXIT_CONFIG,
        format!("ambiguous fixtures (same score, same set): {}", names.join(", ")),
    )
    .with_entry(entry)
}

fn open_log(path: &Path) -> Result<LockedLog, String> {
    LockedLog::open(path).map_err(|e| format!("can't open log {}: {e}", path.display()))
}

fn sleep_ms(ms: u64) {
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// Lossily converts process arguments, dropping argv[0].
pub fn string_args(args: impl IntoIterator<Item = OsString>) -> (OsString, Vec<String>) {
    let mut iter = args.into_iter();
    let program = iter.next().unwrap_or_default();
    (program, iter.map(|a| a.to_string_lossy().into_owned()).collect())
}
