//! Parsing and validating `parity/scenarios/<name>.toml` (format v1, documented in `parity/README.md`).
//!
//! Parsing happens in two stages: serde reads the file into `Raw*` types that mirror the TOML and reject unknown
//! keys, then [`Scenario::parse`] checks the rules serde can't express (exactly one action per step, capture names,
//! key specs, ...) and builds the typed [`Scenario`]. Every error carries the `file:line` it came from.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use fake_gh::fixture::LoggedRequestMatcher;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use toml::Spanned;

use super::keys;

/// The only scenario format version this harness reads.
pub const SCENARIO_VERSION: u32 = 1;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_KEY_DELAY: Duration = Duration::from_millis(30);
const DEFAULT_CAPTURE_STABLE: Duration = Duration::from_millis(300);
/// How long a `wait` step may take when it doesn't set `timeout_ms`.
pub const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

/// One of the two implementations under test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum App {
    Python,
    Rust,
}

impl App {
    pub const ALL: [App; 2] = [App::Python, App::Rust];

    pub fn name(self) -> &'static str {
        match self {
            App::Python => "python",
            App::Rust => "rust",
        }
    }
}

impl fmt::Display for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A terminal size. Written `COLSxROWS`, e.g. `160x48`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Size {
    fn from_pair([cols, rows]: [u16; 2]) -> Option<Self> {
        (cols > 0 && rows > 0).then_some(Self { cols, rows })
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.cols, self.rows)
    }
}

impl FromStr for Size {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || format!("invalid size {s:?}: expected COLSxROWS, e.g. 160x48");
        let (cols, rows) = s.split_once('x').ok_or_else(err)?;
        let cols = cols.parse().map_err(|_| err())?;
        let rows = rows.parse().map_err(|_| err())?;
        Self::from_pair([cols, rows]).ok_or_else(err)
    }
}

/// `env.gh_auth`: what fake-gh's `gh auth status` reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GhAuth {
    #[default]
    LoggedIn,
    LoggedOut,
}

impl GhAuth {
    /// The `FAKE_GH_AUTH` value.
    pub fn env_value(self) -> &'static str {
        match self {
            GhAuth::LoggedIn => "logged-in",
            GhAuth::LoggedOut => "logged-out",
        }
    }
}

/// A structural check a capture can ask for (evaluated by #93).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Check {
    Titles,
    Tabs,
    Headers,
    Rows,
    Cursor,
    Footer,
    Focus,
}

impl Check {
    pub const ALL: [Check; 7] = [
        Check::Titles,
        Check::Tabs,
        Check::Headers,
        Check::Rows,
        Check::Cursor,
        Check::Footer,
        Check::Focus,
    ];
}

/// `ignore` on a capture step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ignore {
    /// Skip every check but `titles` inside boxes whose title contains one of these.
    #[serde(default)]
    pub boxes: Vec<String>,
}

/// `[expect.<capture>]`: expectations checked against each app on its own (evaluated by #93).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub pane_titles: Option<Vec<String>>,
    pub focused: Option<Vec<String>>,
    pub contains: Option<Vec<String>>,
    pub not_contains: Option<Vec<String>>,
    pub footer: Option<Vec<String>>,
    pub tabs: Option<ExpectTabs>,
}

/// `tabs = { box = "Details", names = [...], active = "..." }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectTabs {
    #[serde(rename = "box")]
    pub in_box: String,
    pub names: Option<Vec<String>>,
    pub active: Option<String>,
}

/// The isolated environment a scenario asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvSpec {
    /// Fixture set names under the fixtures root, earliest first (it overrides the later ones).
    pub fixtures: Vec<String>,
    /// The config file name under `parity/configs/`, without `.json`.
    pub config: String,
    pub gh_auth: GhAuth,
    /// `owner/name` for the work dir's `origin`; `None` leaves the work dir outside any git repo.
    pub git_remote: Option<String>,
    pub git_branch: String,
    /// Deep-merged over the config file. Always a JSON object.
    pub config_overrides: Value,
}

/// What a `wait` step waits for.
#[derive(Debug, Clone)]
pub enum WaitFor {
    /// The substring appears on some row.
    Text(String),
    /// The substring is on no row.
    Gone(String),
    /// The pattern matches the screen, rows joined by `\n`.
    Regex(Regex),
    /// The screen's characters haven't changed for this long.
    Stable(Duration),
    /// fake-gh's log has a served request matching this.
    Log(LoggedRequestMatcher),
}

#[derive(Debug, Clone)]
pub struct Wait {
    pub until: WaitFor,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct Capture {
    pub name: String,
    pub checks: Vec<Check>,
    pub ignore: Ignore,
}

/// One `[[step]]`.
#[derive(Debug, Clone)]
pub enum Step {
    Wait(Wait),
    /// Key specs, already validated by [`keys::encode`].
    Keys(Vec<String>),
    Type(String),
    Paste(String),
    Capture(Capture),
    Sleep(Duration),
    Resize(Size),
}

/// Timing settings shared by every step of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// The whole run, per app per size.
    pub timeout: Duration,
    /// The pause after each key (at least [`ESCAPE_DELAY`](super::runner::ESCAPE_DELAY) after `escape`).
    pub key_delay: Duration,
    /// The implicit stability wait before each capture.
    pub capture_stable: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            key_delay: DEFAULT_KEY_DELAY,
            capture_stable: DEFAULT_CAPTURE_STABLE,
        }
    }
}

/// A validated scenario.
#[derive(Debug, Clone)]
pub struct Scenario {
    /// The file stem.
    pub name: String,
    pub path: PathBuf,
    pub description: String,
    pub gate: bool,
    pub apps: Vec<App>,
    pub parity_rows: Vec<String>,
    pub sizes: Vec<Size>,
    pub timing: Timing,
    pub env: EnvSpec,
    pub steps: Vec<Step>,
    pub expect: BTreeMap<String, Expect>,
}

impl Scenario {
    /// The names of the scenario's captures, in step order.
    pub fn capture_names(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().filter_map(|s| match s {
            Step::Capture(c) => Some(c.name.as_str()),
            _ => None,
        })
    }
}

/// An invalid scenario file, located by line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioError {
    pub file: PathBuf,
    /// 1-based; 0 when the problem has no line (e.g. the file couldn't be read).
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file.display(), self.message)
        } else {
            write!(f, "{}:{}: {}", self.file.display(), self.line, self.message)
        }
    }
}

impl std::error::Error for ScenarioError {}

/// Where a scenario's named resources live.
#[derive(Debug, Clone)]
pub struct Roots {
    /// Holds the fixture sets (`parity/fixtures`).
    pub fixtures: PathBuf,
    /// Holds `<name>.json` config files (`parity/configs`).
    pub configs: PathBuf,
}

impl Roots {
    pub fn config_file(&self, name: &str) -> PathBuf {
        self.configs.join(format!("{name}.json"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenario {
    version: Spanned<u32>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    gate: bool,
    apps: Option<Spanned<Vec<App>>>,
    #[serde(default)]
    parity_rows: Vec<String>,
    sizes: Option<Spanned<Vec<[u16; 2]>>>,
    timeout_ms: Option<u64>,
    key_delay_ms: Option<u64>,
    capture_stable_ms: Option<u64>,
    #[serde(default)]
    env: RawEnv,
    #[serde(default, rename = "step")]
    steps: Vec<Spanned<RawStep>>,
    #[serde(default)]
    expect: BTreeMap<Spanned<String>, Expect>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEnv {
    fixtures: Option<Spanned<Vec<String>>>,
    config: Option<Spanned<String>>,
    #[serde(default)]
    gh_auth: GhAuth,
    git_remote: Option<Spanned<String>>,
    git_branch: Option<String>,
    config_overrides: Option<toml::Table>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    wait: Option<Spanned<RawWait>>,
    keys: Option<Vec<Spanned<String>>>,
    #[serde(rename = "type")]
    type_text: Option<String>,
    paste: Option<String>,
    capture: Option<Spanned<String>>,
    checks: Option<Vec<Check>>,
    ignore: Option<Ignore>,
    sleep_ms: Option<u64>,
    resize: Option<Spanned<[u16; 2]>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWait {
    text: Option<String>,
    gone: Option<String>,
    regex: Option<Spanned<String>>,
    stable_ms: Option<u64>,
    log: Option<LoggedRequestMatcher>,
    timeout_ms: Option<u64>,
}

/// Turns byte offsets into `file:line` errors.
struct Locator<'a> {
    file: &'a Path,
    source: &'a str,
}

impl Locator<'_> {
    fn line_of(&self, offset: usize) -> usize {
        let offset = offset.min(self.source.len());
        // Spans from the TOML parser always fall on char boundaries.
        self.source.get(..offset).map_or(0, |s| s.matches('\n').count()) + 1
    }

    fn at(&self, offset: usize, message: impl Into<String>) -> ScenarioError {
        ScenarioError {
            file: self.file.to_owned(),
            line: self.line_of(offset),
            message: message.into(),
        }
    }

    fn at_span<T>(&self, spanned: &Spanned<T>, message: impl Into<String>) -> ScenarioError {
        self.at(spanned.span().start, message)
    }
}

impl Scenario {
    /// Reads and validates a scenario file.
    pub fn load(path: &Path, roots: &Roots) -> Result<Self, ScenarioError> {
        let source = std::fs::read_to_string(path).map_err(|e| ScenarioError {
            file: path.to_owned(),
            line: 0,
            message: format!("can't read: {e}"),
        })?;
        Self::parse(path, &source, roots)
    }

    /// Validates `source`, the contents of the scenario file at `path`.
    pub fn parse(path: &Path, source: &str, roots: &Roots) -> Result<Self, ScenarioError> {
        let loc = Locator { file: path, source };
        let raw: RawScenario = toml::from_str(source).map_err(|e| {
            let line = e.span().map_or(0, |s| loc.line_of(s.start));
            ScenarioError {
                file: path.to_owned(),
                line,
                message: e.message().trim_end().to_owned(),
            }
        })?;

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| loc.at(0, "the file name isn't valid UTF-8"))?
            .to_owned();

        if *raw.version.get_ref() != SCENARIO_VERSION {
            return Err(loc.at_span(
                &raw.version,
                format!(
                    "unsupported version {}; this harness reads version {SCENARIO_VERSION}",
                    raw.version.get_ref()
                ),
            ));
        }

        let apps = match raw.apps {
            None => App::ALL.to_vec(),
            Some(apps) => {
                let unique: BTreeSet<_> = apps.get_ref().iter().collect();
                if apps.get_ref().is_empty() || unique.len() != apps.get_ref().len() {
                    return Err(loc.at_span(&apps, "`apps` must list python and/or rust, each at most once"));
                }
                apps.into_inner()
            }
        };

        let sizes = match raw.sizes {
            None => vec![Size { cols: 160, rows: 48 }],
            Some(sizes) => {
                let parsed: Option<Vec<Size>> = sizes.get_ref().iter().copied().map(Size::from_pair).collect();
                match parsed {
                    Some(parsed) if !parsed.is_empty() => parsed,
                    _ => return Err(loc.at_span(&sizes, "`sizes` must be a non-empty list of [cols, rows] above 0")),
                }
            }
        };

        let timing = Timing {
            timeout: raw.timeout_ms.map_or(DEFAULT_TIMEOUT, Duration::from_millis),
            key_delay: raw.key_delay_ms.map_or(DEFAULT_KEY_DELAY, Duration::from_millis),
            capture_stable: raw
                .capture_stable_ms
                .map_or(DEFAULT_CAPTURE_STABLE, Duration::from_millis),
        };

        let env = parse_env(raw.env, &loc, roots)?;

        let mut steps = Vec::with_capacity(raw.steps.len());
        let mut captures = BTreeSet::new();
        for step in raw.steps {
            let start = step.span().start;
            let step = parse_step(step.into_inner(), start, &loc)?;
            if let Step::Capture(c) = &step
                && !captures.insert(c.name.clone())
            {
                return Err(loc.at(start, format!("duplicate capture name {:?}", c.name)));
            }
            steps.push(step);
        }

        let mut expect = BTreeMap::new();
        for (key, value) in raw.expect {
            if !captures.contains(key.get_ref()) {
                return Err(loc.at_span(
                    &key,
                    format!("[expect.{}] names no capture in this scenario", key.get_ref()),
                ));
            }
            expect.insert(key.into_inner(), value);
        }

        Ok(Self {
            name,
            path: path.to_owned(),
            description: raw.description,
            gate: raw.gate,
            apps,
            parity_rows: raw.parity_rows,
            sizes,
            timing,
            env,
            steps,
            expect,
        })
    }
}

fn parse_env(raw: RawEnv, loc: &Locator<'_>, roots: &Roots) -> Result<EnvSpec, ScenarioError> {
    let fixtures = match raw.fixtures {
        None => vec!["base".to_owned()],
        Some(sets) => {
            if sets.get_ref().is_empty() {
                return Err(loc.at_span(&sets, "`env.fixtures` must name at least one fixture set"));
            }
            for set in sets.get_ref() {
                if !is_plain_name(set) || !roots.fixtures.join(set).is_dir() {
                    return Err(loc.at_span(
                        &sets,
                        format!(
                            "fixture set {set:?} isn't a directory under {}",
                            roots.fixtures.display()
                        ),
                    ));
                }
            }
            sets.into_inner()
        }
    };

    let config = match raw.config {
        None => "default".to_owned(),
        Some(name) => {
            if !is_plain_name(name.get_ref()) {
                return Err(loc.at_span(&name, format!("invalid config name {:?}", name.get_ref())));
            }
            name.into_inner()
        }
    };
    let config_file = roots.config_file(&config);
    if !config_file.is_file() {
        return Err(loc.at(0, format!("config file {} doesn't exist", config_file.display())));
    }

    let git_remote = match raw.git_remote {
        None => None,
        Some(remote) => {
            let valid = remote
                .get_ref()
                .split_once('/')
                .is_some_and(|(owner, name)| is_plain_name(owner) && is_plain_name(name));
            if !valid {
                return Err(loc.at_span(
                    &remote,
                    format!("`git_remote` must be owner/name, got {:?}", remote.get_ref()),
                ));
            }
            Some(remote.into_inner())
        }
    };

    let config_overrides = match raw.config_overrides {
        None => Value::Object(serde_json::Map::new()),
        // TOML values (dates aside, which configs don't use) map one to one onto JSON.
        Some(table) => serde_json::to_value(table).map_err(|e| loc.at(0, format!("env.config_overrides: {e}")))?,
    };

    Ok(EnvSpec {
        fixtures,
        config,
        gh_auth: raw.gh_auth,
        git_remote,
        git_branch: raw.git_branch.unwrap_or_else(|| "main".to_owned()),
        config_overrides,
    })
}

/// A name that's safe as one path segment: `[A-Za-z0-9._-]+`, not `.` or `..`.
fn is_plain_name(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn is_capture_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn parse_step(raw: RawStep, start: usize, loc: &Locator<'_>) -> Result<Step, ScenarioError> {
    if raw.capture.is_none() && (raw.checks.is_some() || raw.ignore.is_some()) {
        return Err(loc.at(start, "`checks` and `ignore` are only allowed in a capture step"));
    }

    let present: Vec<&str> = [
        ("wait", raw.wait.is_some()),
        ("keys", raw.keys.is_some()),
        ("type", raw.type_text.is_some()),
        ("paste", raw.paste.is_some()),
        ("capture", raw.capture.is_some()),
        ("sleep_ms", raw.sleep_ms.is_some()),
        ("resize", raw.resize.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, set)| set.then_some(name))
    .collect();
    match present.as_slice() {
        [] => {
            return Err(loc.at(
                start,
                "a step needs exactly one of wait, keys, type, paste, capture, sleep_ms or resize; it has none",
            ));
        }
        [_] => {}
        more => {
            return Err(loc.at(
                start,
                format!("a step needs exactly one action, but this one has {}", more.join(", ")),
            ));
        }
    }
    if let Some(wait) = raw.wait {
        return parse_wait(wait, loc).map(Step::Wait);
    }
    if let Some(specs) = raw.keys {
        for spec in &specs {
            keys::encode(spec.get_ref(), false).map_err(|e| loc.at_span(spec, e.to_string()))?;
        }
        return Ok(Step::Keys(specs.into_iter().map(Spanned::into_inner).collect()));
    }
    if let Some(text) = raw.type_text {
        return Ok(Step::Type(text));
    }
    if let Some(text) = raw.paste {
        return Ok(Step::Paste(text));
    }
    if let Some(name) = raw.capture {
        if !is_capture_name(name.get_ref()) {
            return Err(loc.at_span(
                &name,
                format!("capture name {:?} must match [a-z0-9-]+", name.get_ref()),
            ));
        }
        return Ok(Step::Capture(Capture {
            name: name.into_inner(),
            checks: raw.checks.unwrap_or_else(|| Check::ALL.to_vec()),
            ignore: raw.ignore.unwrap_or_default(),
        }));
    }
    if let Some(ms) = raw.sleep_ms {
        return Ok(Step::Sleep(Duration::from_millis(ms)));
    }
    if let Some(size) = raw.resize {
        return Size::from_pair(*size.get_ref())
            .map(Step::Resize)
            .ok_or_else(|| loc.at_span(&size, "`resize` must be [cols, rows] above 0"));
    }
    unreachable!("exactly one action is present")
}

fn parse_wait(spanned: Spanned<RawWait>, loc: &Locator<'_>) -> Result<Wait, ScenarioError> {
    let start = spanned.span().start;
    let raw = spanned.into_inner();
    let count = [
        raw.text.is_some(),
        raw.gone.is_some(),
        raw.regex.is_some(),
        raw.stable_ms.is_some(),
        raw.log.is_some(),
    ]
    .into_iter()
    .filter(|&b| b)
    .count();
    if count != 1 {
        return Err(loc.at(start, "`wait` needs exactly one of text, gone, regex, stable_ms or log"));
    }
    let until = if let Some(text) = raw.text {
        WaitFor::Text(text)
    } else if let Some(text) = raw.gone {
        WaitFor::Gone(text)
    } else if let Some(pattern) = raw.regex {
        let re = Regex::new(pattern.get_ref()).map_err(|e| loc.at_span(&pattern, format!("invalid regex: {e}")))?;
        WaitFor::Regex(re)
    } else if let Some(ms) = raw.stable_ms {
        WaitFor::Stable(Duration::from_millis(ms))
    } else if let Some(log) = raw.log {
        WaitFor::Log(log)
    } else {
        unreachable!("exactly one condition is present")
    };
    Ok(Wait {
        until,
        timeout: raw.timeout_ms.map_or(DEFAULT_WAIT_TIMEOUT, Duration::from_millis),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        let parity = Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity");
        Roots {
            fixtures: parity.join("fixtures"),
            configs: parity.join("configs"),
        }
    }

    fn parse(source: &str) -> Result<Scenario, ScenarioError> {
        Scenario::parse(Path::new("parity/scenarios/test.toml"), source, &roots())
    }

    /// Asserts that `source` fails on `line` with a message containing `needle`.
    #[track_caller]
    fn assert_error(source: &str, line: usize, needle: &str) {
        let err = parse(source).expect_err("scenario should be invalid");
        assert_eq!(err.line, line, "{err}");
        assert!(err.message.contains(needle), "{err:?} should mention {needle:?}");
        assert!(
            err.to_string()
                .starts_with(&format!("parity/scenarios/test.toml:{line}: ")),
            "{err}"
        );
    }

    const VALID: &str = r#"
version = 1
description = "Main screen"
gate = true
apps = ["python"]
parity_rows = ["Main layout"]
sizes = [[160, 48], [100, 30]]
timeout_ms = 20000
key_delay_ms = 10
capture_stable_ms = 200

[env]
fixtures = ["base"]
config = "default"
gh_auth = "logged-out"
git_remote = "gizmo385/gh-lazy"
git_branch = "dev"

[env.config_overrides]
appearance = { show_command_log = true }

[[step]]
wait = { text = "Repositories", timeout_ms = 5000 }

[[step]]
capture = "main"

[[step]]
keys = ["j", "ctrl+p", "escape"]

[[step]]
wait = { stable_ms = 500 }

[[step]]
type = "gh-lazy"

[[step]]
paste = "pasted"

[[step]]
sleep_ms = 50

[[step]]
resize = [100, 30]

[[step]]
wait = { regex = "^\\s*Repo", timeout_ms = 100 }

[[step]]
wait = { log = { method = "GET", path = "/user/repos" } }

[[step]]
wait = { gone = "Loading" }

[[step]]
capture = "after-select"
checks = ["titles", "footer"]
ignore = { boxes = ["Details"] }

[expect.main]
pane_titles = ["", "[1] Repositories"]
focused = ["[1] Repositories"]
contains = ["Repositories"]
not_contains = ["Traceback"]
footer = ["q Quit"]
tabs = { box = "Details", names = ["Info"], active = "Info" }
"#;

    #[test]
    fn parses_a_valid_scenario() {
        let s = parse(VALID).unwrap();
        assert_eq!(s.name, "test");
        assert!(s.gate);
        assert_eq!(s.apps, [App::Python]);
        assert_eq!(s.sizes, [Size { cols: 160, rows: 48 }, Size { cols: 100, rows: 30 }]);
        assert_eq!(
            s.timing,
            Timing {
                timeout: Duration::from_secs(20),
                key_delay: Duration::from_millis(10),
                capture_stable: Duration::from_millis(200)
            }
        );
        assert_eq!(s.env.gh_auth, GhAuth::LoggedOut);
        assert_eq!(s.env.git_remote.as_deref(), Some("gizmo385/gh-lazy"));
        assert_eq!(s.env.git_branch, "dev");
        assert_eq!(
            s.env.config_overrides,
            serde_json::json!({"appearance": {"show_command_log": true}})
        );
        assert_eq!(s.steps.len(), 12);
        assert!(
            matches!(&s.steps[0], Step::Wait(Wait { until: WaitFor::Text(t), timeout })
            if t == "Repositories" && *timeout == Duration::from_secs(5))
        );
        assert!(matches!(&s.steps[2], Step::Keys(k) if k == &["j", "ctrl+p", "escape"]));
        assert!(
            matches!(&s.steps[3], Step::Wait(Wait { until: WaitFor::Stable(d), timeout })
            if *d == Duration::from_millis(500) && *timeout == DEFAULT_WAIT_TIMEOUT)
        );
        assert!(matches!(&s.steps[7], Step::Resize(Size { cols: 100, rows: 30 })));
        assert!(matches!(&s.steps[9], Step::Wait(Wait { until: WaitFor::Log(m), .. }) if m.path == "/user/repos"));
        let Step::Capture(c) = &s.steps[1] else {
            panic!("not a capture")
        };
        assert_eq!(c.checks, Check::ALL);
        let Step::Capture(c) = &s.steps[11] else {
            panic!("not a capture")
        };
        assert_eq!(c.checks, [Check::Titles, Check::Footer]);
        assert_eq!(c.ignore.boxes, ["Details"]);
        assert_eq!(s.capture_names().collect::<Vec<_>>(), ["main", "after-select"]);
        assert_eq!(s.expect["main"].tabs.as_ref().unwrap().in_box, "Details");
    }

    #[test]
    fn applies_defaults() {
        let s = parse("version = 1\n").unwrap();
        assert!(!s.gate);
        assert_eq!(s.apps, App::ALL);
        assert_eq!(s.sizes, [Size { cols: 160, rows: 48 }]);
        assert_eq!(s.timing, Timing::default());
        assert_eq!(s.env.fixtures, ["base"]);
        assert_eq!(s.env.config, "default");
        assert_eq!(s.env.gh_auth, GhAuth::LoggedIn);
        assert_eq!(s.env.git_remote, None);
        assert_eq!(s.env.git_branch, "main");
    }

    #[test]
    fn rejects_two_actions_in_one_step() {
        assert_error(
            "version = 1\n\n[[step]]\nkeys = [\"j\"]\ncapture = \"x\"\n",
            3,
            "keys, capture",
        );
    }

    #[test]
    fn rejects_a_step_with_no_action() {
        assert_error(
            "version = 1\n[[step]]\nsleep_ms = 1\n[[step]]\n\n[[step]]\nsleep_ms = 1\n",
            4,
            "it has none",
        );
        assert_error(
            "version = 1\n[[step]]\nchecks = [\"titles\"]\n",
            2,
            "only allowed in a capture step",
        );
    }

    #[test]
    fn rejects_unknown_keys() {
        assert_error(
            "version = 1\n\n[[step]]\nwait = { txt = \"x\" }\n",
            4,
            "unknown field `txt`",
        );
        assert_error("version = 1\nsize = [[1, 1]]\n", 2, "unknown field `size`");
        assert_error("version = 1\n[[step]]\npress = [\"j\"]\n", 3, "unknown field `press`");
    }

    #[test]
    fn rejects_expect_for_a_missing_capture() {
        assert_error(
            "version = 1\n[[step]]\ncapture = \"main\"\n\n[expect.mian]\ncontains = [\"x\"]\n",
            5,
            "[expect.mian] names no capture",
        );
    }

    #[test]
    fn rejects_duplicate_capture_names() {
        assert_error(
            "version = 1\n[[step]]\ncapture = \"a\"\n[[step]]\ncapture = \"a\"\n",
            4,
            "duplicate capture",
        );
    }

    #[test]
    fn rejects_indistinct_keys() {
        assert_error("version = 1\n[[step]]\nkeys = [\"j\",\n  \"ctrl+m\"]\n", 4, "ctrl+m");
        assert_error("version = 1\n[[step]]\nkeys = [\"hyper+x\"]\n", 3, "unknown key");
    }

    #[test]
    fn rejects_other_invalid_values() {
        assert_error("version = 2\n", 1, "unsupported version 2");
        assert_error("version = 1\n[[step]]\ncapture = \"Main\"\n", 3, "[a-z0-9-]+");
        assert_error(
            "version = 1\n[[step]]\nwait = { text = \"a\", gone = \"b\" }\n",
            3,
            "exactly one of",
        );
        assert_error(
            "version = 1\n[[step]]\nwait = { timeout_ms = 5 }\n",
            3,
            "exactly one of",
        );
        assert_error("version = 1\n[[step]]\nwait = { regex = \"(\" }\n", 3, "invalid regex");
        assert_error("version = 1\nsizes = [[0, 48]]\n", 2, "sizes");
        assert_error("version = 1\napps = [\"python\", \"python\"]\n", 2, "apps");
        assert_error("version = 1\napps = [\"go\"]\n", 2, "unknown variant `go`");
        assert_error("version = 1\n[env]\nfixtures = [\"no-such-set\"]\n", 3, "no-such-set");
        assert_error("version = 1\n[env]\nfixtures = [\"../base\"]\n", 3, "../base");
        assert_error("version = 1\n[env]\ngit_remote = \"just-a-name\"\n", 3, "owner/name");
        assert_error(
            "version = 1\n[env]\ngh_auth = \"maybe\"\n",
            3,
            "unknown variant `maybe`",
        );
        assert_error("version = 1\n[[step]]\nresize = [0, 1]\n", 3, "resize");
        assert_error(
            "version = 1\n[[step]]\ncapture = \"a\"\n[expect.a]\npane_title = []\n",
            5,
            "pane_title",
        );
    }

    #[test]
    fn reports_a_missing_config_file() {
        let err = parse("version = 1\n[env]\nconfig = \"nope\"\n").unwrap_err();
        assert!(err.message.contains("nope.json"), "{err}");
    }

    #[test]
    fn committed_scenarios_are_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity/scenarios");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "toml") {
                Scenario::load(&path, &roots()).unwrap_or_else(|e| panic!("{e}"));
                count += 1;
            }
        }
        assert!(count > 0, "no scenarios found");
    }

    #[test]
    fn sizes_parse_from_the_command_line_form() {
        assert_eq!("160x48".parse(), Ok(Size { cols: 160, rows: 48 }));
        assert!("160".parse::<Size>().is_err());
        assert!("0x48".parse::<Size>().is_err());
        assert!("axb".parse::<Size>().is_err());
        assert_eq!(Size { cols: 100, rows: 30 }.to_string(), "100x30");
    }
}
