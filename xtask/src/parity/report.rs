//! The human-readable report, written at the end of every run from `summary.json` and the dumps on disk:
//! `<capture>.diff.txt` per size, a self-contained `report.html` per scenario, `index.html` and `summary.md`.
//!
//! `check.json` and `structure.json` (#93) are read when they exist; without them the report says "not checked".
//! The check writes no `check.json` for a capture it had nothing to check, so "not checked" covers that too.
//! Everything here is plain string building: the markup is small and fixed, so there's no template engine.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use fake_gh::log::LogEntry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::diff::text_diff;
use super::grid::{Grid, Style};
use super::runner::{AppRunStatus, RunState};
use super::scenario::App;
use super::summary::{ScenarioResult, ScenarioSummary, SizeSummary, Summary};
use super::{Context, Result};

/// The terminal's default foreground, for cells whose fg is `null`.
pub const DEFAULT_FG: &str = "#d4d4d4";
/// The terminal's default background, for cells whose bg is `null`.
pub const DEFAULT_BG: &str = "#1e1e1e";
/// How much of each app's `fake-gh.log` the report shows.
const LOG_TAIL_LINES: usize = 50;
/// How many mismatches `summary.md` lists.
const SUMMARY_MISMATCHES: usize = 20;
/// Mismatch values in `summary.md` are cut to this many characters.
const SUMMARY_VALUE_CHARS: usize = 80;
/// The `scenario/size/capture` and `check box` parts of a mismatch line are cut to this many characters.
const SUMMARY_TARGET_CHARS: usize = 120;
/// Ends the `summary.md` mismatch list when the next line would cross [`SUMMARY_MD_LIMIT`].
const MORE_MISMATCHES: &str = "- …the rest are in index.html.\n";
/// `summary.md` stays under this. GitHub allows 1 MiB per job summary; this leaves room for other steps.
const SUMMARY_MD_LIMIT: usize = 64 * 1024;
/// The size columns `summary.md` always has, in this order. Other sizes in the run are appended.
const SUMMARY_SIZES: [&str; 2] = ["160x48", "100x30"];

/// Writing to a `String` can't fail.
macro_rules! w {
    ($dst:expr, $($arg:tt)*) => {{
        let _ = write!($dst, $($arg)*);
    }};
}

/// The file [`write_reports`] writes [`RunMeta`] to, next to `summary.md`, for tools that publish the report
/// (`cargo xtask parity-site`).
pub const META_FILE: &str = "meta.json";

/// Where and how the run happened, shown at the top of `index.html` and written to [`META_FILE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMeta {
    pub git_sha: String,
    /// The commit's subject line, empty when git couldn't say.
    #[serde(default)]
    pub subject: String,
    pub timestamp: String,
    pub os: String,
    /// The command line after `cargo xtask`, e.g. `parity startup --python-only`.
    pub command: String,
    /// Where the Python side came from.
    pub python: AppSource,
    /// Whether the Rust app ran: [`AppSource::Live`] or [`AppSource::NotRun`].
    pub rust: AppSource,
}

/// What a run did with one app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppSource {
    /// A flag left the app out.
    NotRun,
    /// Read from recorded references; only the Python side can be.
    Recorded,
    /// Run live in every scenario.
    Live,
    /// Run live in these scenarios and read from references in the rest.
    LiveFor(Vec<String>),
}

impl std::fmt::Display for AppSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppSource::NotRun => f.write_str("not run"),
            AppSource::Recorded => f.write_str("recorded"),
            AppSource::Live => f.write_str("live"),
            AppSource::LiveFor(names) => write!(f, "live for {}; recorded for the rest", names.join(", ")),
        }
    }
}

impl RunMeta {
    /// The current run: `git rev-parse HEAD` in `repo`, the time now and this process's arguments.
    pub fn current(repo: &Path, python: AppSource, rust: AppSource) -> Self {
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(repo)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        };
        Self {
            git_sha: git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_owned()),
            subject: git(&["log", "-1", "--format=%s", "HEAD"]).unwrap_or_default(),
            timestamp: jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
            os: format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH),
            command: std::env::args().skip(1).collect::<Vec<_>>().join(" "),
            python,
            rust,
        }
    }

    /// Reads the [`META_FILE`] a run wrote into the report directory `dir`.
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join(META_FILE);
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Why `app` has no run at a size.
    fn not_run(&self, app: App) -> &'static str {
        match app {
            App::Rust if self.rust == AppSource::NotRun => "not run (--python-only)",
            App::Python if self.python == AppSource::NotRun => "not run (--rust-only)",
            _ => "not run (not in the scenario's apps)",
        }
    }

    /// `Python: recorded`, `Python: live`, ...
    pub fn python_line(&self) -> String {
        format!("Python: {}", self.python)
    }
}

/// Writes [`META_FILE`], every `<capture>.diff.txt`, each scenario's `report.html`, `index.html` and `summary.md`
/// into `out`.
pub fn write_reports(out: &Path, summary: &Summary, meta: &RunMeta) -> Result<()> {
    // Plain strings and enums; serializing can't fail.
    let json = serde_json::to_string_pretty(meta).expect("run metadata serializes");
    std::fs::write(out.join(META_FILE), json + "\n").with_context(|| format!("writing {META_FILE}"))?;
    let mut scenarios = Vec::with_capacity(summary.scenarios.len());
    for scenario in &summary.scenarios {
        let data = load_scenario(out, scenario)?;
        let dir = out.join(&scenario.name);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        std::fs::write(dir.join("report.html"), render_scenario(&data, meta))
            .with_context(|| format!("writing {}/report.html", scenario.name))?;
        scenarios.push(data);
    }
    std::fs::write(out.join("index.html"), render_index(&scenarios, meta)).context("writing index.html")?;
    std::fs::write(out.join("summary.md"), render_summary_md(&scenarios, meta)).context("writing summary.md")?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Loading

/// One value per app.
#[derive(Debug, Clone, Default)]
struct Pair<T> {
    python: T,
    rust: T,
}

impl<T> Pair<T> {
    fn try_from_fn(mut f: impl FnMut(App) -> Result<T>) -> Result<Self> {
        Ok(Self {
            python: f(App::Python)?,
            rust: f(App::Rust)?,
        })
    }

    fn get(&self, app: App) -> &T {
        match app {
            App::Python => &self.python,
            App::Rust => &self.rust,
        }
    }
}

/// One structural mismatch from `check.json` (#93). Every field is optional so a report never fails on a newer
/// or partial file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Mismatch {
    check: String,
    #[serde(rename = "box")]
    box_title: Value,
    python: Value,
    rust: Value,
    /// What `[expect]` wanted; `null` for a cross-app mismatch.
    expected: Value,
    message: String,
}

/// `<capture>.check.json`: `expect` mismatches are per app, `cross` ones are Python vs Rust, and `drift` ones are a
/// live Python run against its reference.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct CheckFile {
    expect: Vec<Mismatch>,
    cross: Vec<Mismatch>,
    drift: Vec<Mismatch>,
}

impl CheckFile {
    fn mismatches(&self) -> impl Iterator<Item = (&'static str, &Mismatch)> {
        let expect = self.expect.iter().map(|m| ("expect", m));
        let cross = self.cross.iter().map(|m| ("cross", m));
        expect.chain(cross).chain(self.drift.iter().map(|m| ("drift", m)))
    }

    fn len(&self) -> usize {
        self.expect.len() + self.cross.len() + self.drift.len()
    }
}

#[derive(Debug, Clone)]
enum CheckState {
    /// No `check.json`: the structural check didn't run.
    Absent,
    Unreadable(String),
    Loaded(CheckFile),
}

impl CheckState {
    fn count(&self) -> Option<usize> {
        match self {
            CheckState::Loaded(c) => Some(c.len()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct CaptureData {
    name: String,
    grids: Pair<Option<Grid>>,
    /// Pretty-printed `structure.json` per app.
    structures: Pair<Option<String>>,
    check: CheckState,
    /// The text diff, when both apps captured.
    diff: Option<String>,
}

#[derive(Debug, Clone)]
struct SizeData<'a> {
    summary: &'a SizeSummary,
    captures: Vec<CaptureData>,
    /// `failure.grid.json` per app.
    failures: Pair<Option<Grid>>,
    /// The last [`LOG_TAIL_LINES`] lines of each app's `fake-gh.log`.
    logs: Pair<Option<String>>,
}

impl SizeData<'_> {
    /// Total mismatches, or `None` when no capture at this size was checked.
    fn mismatches(&self) -> Option<usize> {
        self.captures
            .iter()
            .filter_map(|c| c.check.count())
            .reduce(|a, b| a + b)
    }

    /// This size's own result word. The scenario result is authoritative; a size inherits a failing scenario
    /// result when it has mismatches of its own, so a size isn't shown as passing when its checks failed.
    fn result(&self, scenario: &ScenarioSummary) -> ScenarioResult {
        let own = ScenarioResult::from_statuses(scenario.gate, std::slice::from_ref(self.summary));
        let inherits = self.mismatches().unwrap_or(0) > 0 && severity(scenario.result) > severity(own);
        if inherits { scenario.result } else { own }
    }
}

#[derive(Debug, Clone)]
struct ScenarioData<'a> {
    summary: &'a ScenarioSummary,
    sizes: Vec<SizeData<'a>>,
}

impl ScenarioData<'_> {
    fn mismatches(&self) -> Option<usize> {
        self.sizes.iter().filter_map(SizeData::mismatches).reduce(|a, b| a + b)
    }
}

fn severity(result: ScenarioResult) -> u8 {
    match result {
        ScenarioResult::Fail => 2,
        ScenarioResult::RustNotReady => 1,
        ScenarioResult::Pass | ScenarioResult::PythonOnly | ScenarioResult::RustOnly => 0,
    }
}

/// Reads a file that may legitimately be missing.
fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn read_grid(path: &Path) -> Result<Option<Grid>> {
    read_optional(path)?
        .map(|text| serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display())))
        .transpose()
}

fn pretty_json(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| text.to_owned())
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Reads one scenario's dumps and writes its `<capture>.diff.txt` files.
fn load_scenario<'a>(out: &Path, scenario: &'a ScenarioSummary) -> Result<ScenarioData<'a>> {
    let dir = out.join(&scenario.name);
    let mut sizes = Vec::with_capacity(scenario.sizes.len());
    for size in &scenario.sizes {
        let size_dir = dir.join(&size.size);
        let ran = |app: App| size.apps.contains_key(&app);
        let app_file = |app: App, file: &str| size_dir.join(app.name()).join(file);

        let failures = Pair::try_from_fn(|app| {
            if ran(app) {
                read_grid(&app_file(app, "failure.grid.json"))
            } else {
                Ok(None)
            }
        })?;
        let logs = Pair::try_from_fn(|app| {
            Ok(if ran(app) {
                read_optional(&app_file(app, "fake-gh.log"))?.map(|t| tail(&t, LOG_TAIL_LINES))
            } else {
                None
            })
        })?;

        let mut captures = Vec::with_capacity(size.captures.len());
        for capture in &size.captures {
            let name = &capture.name;
            let grids = Pair::try_from_fn(|app| {
                if ran(app) {
                    read_grid(&app_file(app, &format!("{name}.grid.json")))
                } else {
                    Ok(None)
                }
            })?;
            // #93's files never fail the report: any read or parse problem is shown on the page instead.
            let structures = Pair::try_from_fn(|app| {
                Ok(if ran(app) {
                    match read_optional(&app_file(app, &format!("{name}.structure.json"))) {
                        Ok(text) => text.map(|t| pretty_json(&t)),
                        Err(e) => Some(format!("structure.json couldn't be read: {e}")),
                    }
                } else {
                    None
                })
            })?;
            let check = match read_optional(&size_dir.join(format!("{name}.check.json"))) {
                Ok(None) => CheckState::Absent,
                Ok(Some(text)) => match serde_json::from_str::<CheckFile>(&text) {
                    Ok(check) => CheckState::Loaded(check),
                    Err(e) => CheckState::Unreadable(e.to_string()),
                },
                Err(e) => CheckState::Unreadable(e.to_string()),
            };
            let diff = match (&grids.python, &grids.rust) {
                (Some(py), Some(rs)) => {
                    let text = |app: App, grid: &Grid| -> Result<String> {
                        Ok(read_optional(&app_file(app, &format!("{name}.txt")))?.unwrap_or_else(|| grid.to_text()))
                    };
                    let diff = text_diff(&text(App::Python, py)?, &text(App::Rust, rs)?);
                    let path = size_dir.join(format!("{name}.diff.txt"));
                    std::fs::write(&path, &diff).with_context(|| format!("writing {}", path.display()))?;
                    Some(diff)
                }
                _ => None,
            };
            captures.push(CaptureData {
                name: name.clone(),
                grids,
                structures,
                check,
                diff,
            });
        }
        sizes.push(SizeData {
            summary: size,
            captures,
            failures,
            logs,
        });
    }
    Ok(ScenarioData {
        summary: scenario,
        sizes,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Grids as HTML

/// HTML-escapes text for element content and double- or single-quoted attributes.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A grid color as CSS, or `None` for the terminal default. Anything that isn't `#rrggbb` or `idx:<0-255>` is
/// treated as the default, so a dump can never inject CSS.
fn css_color(color: &str) -> Option<String> {
    if let Some(hex) = color.strip_prefix('#') {
        return (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| format!("#{}", hex.to_ascii_lowercase()));
    }
    color.strip_prefix("idx:")?.parse::<u8>().ok().map(xterm_color)
}

/// The xterm 256-color palette.
fn xterm_color(n: u8) -> String {
    const BASE: [&str; 16] = [
        "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5", "#7f7f7f", "#ff0000",
        "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
    ];
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match n {
        0..=15 => BASE[usize::from(n)].to_owned(),
        16..=231 => {
            let i = n - 16;
            let level = |v: u8| LEVELS[usize::from(v)];
            format!("#{:02x}{:02x}{:02x}", level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let v = 8 + (n - 232) * 10;
            format!("#{v:02x}{v:02x}{v:02x}")
        }
    }
}

/// The CSS for a cell style, with inverse applied and the default colors left out (the `<pre>` has them).
fn style_css(style: &Style) -> String {
    let mut fg = style.fg.as_deref().and_then(css_color);
    let mut bg = style.bg.as_deref().and_then(css_color);
    if style.inverse {
        let old_fg = fg.take().unwrap_or_else(|| DEFAULT_FG.to_owned());
        fg = Some(bg.take().unwrap_or_else(|| DEFAULT_BG.to_owned()));
        bg = Some(old_fg);
    }
    let mut css = String::new();
    if let Some(fg) = fg.filter(|c| c != DEFAULT_FG) {
        w!(css, "color:{fg};");
    }
    if let Some(bg) = bg.filter(|c| c != DEFAULT_BG) {
        w!(css, "background:{bg};");
    }
    if style.bold {
        css.push_str("font-weight:bold;");
    }
    if style.italic {
        css.push_str("font-style:italic;");
    }
    if style.underline {
        css.push_str("text-decoration:underline;");
    }
    css
}

/// The distinct cell styles on one page, emitted once as `.s<n>` classes so a full screen stays small.
#[derive(Debug, Default)]
struct StyleSheet {
    classes: Vec<String>,
    index: HashMap<String, usize>,
}

impl StyleSheet {
    /// The class for a style, or `None` when it's all defaults.
    fn class(&mut self, style: &Style) -> Option<usize> {
        let css = style_css(style);
        if css.is_empty() {
            return None;
        }
        if let Some(&i) = self.index.get(&css) {
            return Some(i);
        }
        let i = self.classes.len();
        self.classes.push(css.clone());
        self.index.insert(css, i);
        Some(i)
    }

    fn css(&self) -> String {
        self.classes
            .iter()
            .enumerate()
            .fold(String::new(), |mut out, (i, css)| {
                w!(out, ".s{i}{{{css}}}\n");
                out
            })
    }
}

/// Renders a grid as a `<pre>` of spans. With `other`, every cell whose text differs from the other grid's cell
/// at the same position gets the `d` class. Returns the HTML and the number of differing cells.
fn render_grid(grid: &Grid, other: Option<&Grid>, sheet: &mut StyleSheet) -> (String, usize) {
    let default_style = Style::default();
    let mut html = String::from("<pre class=\"term\">");
    let mut differing = 0;
    for (row, line) in grid.lines.iter().enumerate() {
        let mut styles = vec![&default_style; line.cells.len()];
        for run in &line.runs {
            let start = usize::from(run.col);
            for slot in styles.iter_mut().skip(start).take(usize::from(run.len)) {
                *slot = &run.style;
            }
        }
        // Adjacent cells with the same class and diff flag share one span.
        let mut current: Option<(Option<usize>, bool)> = None;
        let mut text = String::new();
        let flush = |html: &mut String, current: Option<(Option<usize>, bool)>, text: &mut String| {
            if text.is_empty() {
                return;
            }
            match current {
                Some((None, false)) | None => html.push_str(text),
                Some((class, diff)) => {
                    html.push_str("<span class=\"");
                    if let Some(i) = class {
                        w!(html, "s{i}");
                    }
                    if diff {
                        html.push_str(if class.is_some() { " d" } else { "d" });
                    }
                    w!(html, "\">{text}</span>");
                }
            }
            text.clear();
        };
        for (col, cell) in line.cells.iter().enumerate() {
            // A wide character's continuation cell has no text; the character before it covers it.
            if cell.is_empty() {
                continue;
            }
            let diff = other.is_some_and(|o| o.lines.get(row).and_then(|l| l.cells.get(col)) != Some(cell));
            differing += usize::from(diff);
            let key = (sheet.class(styles[col]), diff);
            if current != Some(key) {
                flush(&mut html, current, &mut text);
                current = Some(key);
            }
            text.push_str(&escape(cell));
        }
        flush(&mut html, current, &mut text);
        html.push('\n');
    }
    html.push_str("</pre>");
    (html, differing)
}

// ---------------------------------------------------------------------------------------------------------------
// Pages

/// The stylesheet every report page shares, light and dark. `cargo xtask parity-site` reuses it so the landing page
/// and the report look like one site.
pub(crate) const PAGE_CSS: &str = r#":root{color-scheme:light dark;--fg:#1f2328;--bg:#ffffff;--muted:#59636e;--line:#d1d9e0;--panel:#f6f8fa}
@media (prefers-color-scheme:dark){:root{--fg:#e6edf3;--bg:#0d1117;--muted:#9198a1;--line:#3d444d;--panel:#151b23}}
*{box-sizing:border-box}
body{margin:0;padding:16px 24px 48px;font:14px/1.5 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;color:var(--fg);background:var(--bg)}
h1{font-size:22px;margin:0 0 4px}h2{font-size:18px;margin:32px 0 8px;padding-top:8px;border-top:1px solid var(--line)}
h3{font-size:15px;margin:20px 0 8px}h4{font-size:13px;margin:16px 0 6px}
a{color:#0969da}@media (prefers-color-scheme:dark){a{color:#4493f8}}
.muted{color:var(--muted)}
code,pre.log{font-family:ui-monospace,"SF Mono",Menlo,"DejaVu Sans Mono",Consolas,monospace;font-size:12px}
table{border-collapse:collapse;margin:8px 0}th,td{border:1px solid var(--line);padding:3px 8px;text-align:left;vertical-align:top}
th{background:var(--panel)}
.badge{display:inline-block;padding:0 8px;border-radius:10px;font-size:12px;font-weight:600;color:#fff;background:#6e7781;white-space:nowrap}
.b-pass{background:#1a7f37}.b-fail{background:#cf222e}.b-rust-not-ready{background:#9a6700}
.b-python-only,.b-rust-only{background:#0969da}
.panels{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px}
.panel{min-width:0;margin:0}.panel figcaption{font-weight:600;margin-bottom:4px}
.scroll{overflow-x:auto;width:fit-content;max-width:100%;border:1px solid var(--line)}
pre.term{margin:0;width:max-content;font-family:ui-monospace,"SF Mono",Menlo,"DejaVu Sans Mono",Consolas,monospace;font-size:12px;line-height:1.2;color:#d4d4d4;background:#1e1e1e}
/* inline-block so a span's background fills the whole 1.2 line height, with no stripes between rows */
pre.term span{display:inline-block;vertical-align:top}
.hl pre.term .d{outline:1px solid #ff4d4f;outline-offset:-1px;background-image:linear-gradient(rgba(255,77,79,.25),rgba(255,77,79,.25))}
.reason{margin:0 0 6px;padding:4px 8px;border-left:3px solid #cf222e;background:var(--panel)}
.notrun{padding:24px;border:1px dashed var(--line);color:var(--muted)}
details{margin:6px 0}summary{cursor:pointer}
pre.log{margin:4px 0;padding:8px;overflow-x:auto;background:var(--panel);border:1px solid var(--line);max-height:480px}
.toolbar{display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin:4px 0 8px}
"#;

/// Under 100 lines, per #94: un-hides the JS-only controls, flips the highlight class and opens or closes every
/// collapsible section. Collapsing itself is native `<details>`, so the page works without JS.
const PAGE_JS: &str = r"for (const el of document.querySelectorAll('[data-js]')) el.hidden = false;
for (const box of document.querySelectorAll('input[data-hl]')) {
  box.addEventListener('change', () => box.closest('.capture').classList.toggle('hl', box.checked));
}
for (const button of document.querySelectorAll('button[data-open]')) {
  button.addEventListener('click', () => {
    for (const d of document.querySelectorAll('details')) d.open = button.dataset.open === '1';
  });
}
";

fn page(title: &str, styles: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n\
         <style>\n{PAGE_CSS}{styles}</style>\n</head>\n<body>\n{body}<script>\n{PAGE_JS}</script>\n</body>\n</html>\n",
        escape(title)
    )
}

pub(crate) fn badge(word: &str) -> String {
    format!("<span class=\"badge b-{0}\">{0}</span>", escape(word))
}

fn state_name(state: RunState) -> &'static str {
    match state {
        RunState::Ok => "ok",
        RunState::Timeout => "timeout",
        RunState::Exited => "exited",
        RunState::SpawnError => "spawn-error",
        RunState::StepFailed => "step-failed",
    }
}

/// A fake-gh miss as a fixture key: `GET /path?name=value`.
fn miss_key(entry: &LogEntry) -> String {
    let mut key = format!(
        "{} {}",
        entry.method.as_deref().unwrap_or("?"),
        entry.path.as_deref().unwrap_or("?")
    );
    if let Some(query) = entry.query.as_ref().filter(|q| !q.is_empty()) {
        let mut encoder = form_urlencoded::Serializer::new(String::new());
        for (name, value) in query {
            for v in value.values() {
                encoder.append_pair(name, v);
            }
        }
        w!(key, "?{}", encoder.finish());
    }
    key
}

fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A mismatch's Python or Rust value. `null` (the app had nothing there) is spelled out.
fn mismatch_value(value: &Value) -> String {
    if value.is_null() {
        "(none)".to_owned()
    } else {
        value_text(value)
    }
}

fn anchor(size: &str, capture: &str) -> String {
    escape(&format!("{size}-{capture}"))
}

fn render_scenario(data: &ScenarioData<'_>, meta: &RunMeta) -> String {
    let s = data.summary;
    let mut sheet = StyleSheet::default();
    let mut body = String::new();

    w!(
        body,
        "<p class=\"muted\"><a href=\"../index.html\">All scenarios</a></p>\n<h1>{} {}</h1>\n",
        escape(&s.name),
        badge(s.result.as_str())
    );
    if !s.description.is_empty() {
        w!(body, "<p>{}</p>\n", escape(&s.description));
    }
    w!(
        body,
        "<p>Gate: <strong>{}</strong>{}</p>\n",
        if s.gate { "yes" } else { "no" },
        if s.gate {
            ""
        } else {
            " (Rust failures are reported as rust-not-ready)"
        }
    );
    if !s.parity_rows.is_empty() {
        body.push_str("<p>Parity matrix rows:</p>\n<ul>\n");
        // The report is self-contained and works from a downloaded artifact, so name the file rather than link it.
        for row in &s.parity_rows {
            match crate::matrix::row_file(row) {
                Some(file) => w!(
                    body,
                    "<li><code>{}</code> in <code>{}</code></li>\n",
                    escape(row),
                    escape(&file)
                ),
                None => w!(body, "<li>{}</li>\n", escape(row)),
            }
        }
        body.push_str("</ul>\n");
    }
    if let Some(error) = &s.error {
        w!(body, "<p class=\"reason\">Harness error: {}</p>\n", escape(error));
    }

    body.push_str(
        "<table>\n<tr><th>Size</th><th>App</th><th>Status</th><th>Duration</th><th>Flaky</th>\
         <th>Failed step</th><th>Exit code</th><th>Fixture misses</th></tr>\n",
    );
    for size in &data.sizes {
        for app in App::ALL {
            w!(body, "<tr><td>{}</td><td>{app}</td>", escape(&size.summary.size));
            match size.summary.apps.get(&app) {
                Some(status) => body.push_str(&status_cells(status)),
                None => w!(body, "<td colspan=\"6\" class=\"muted\">{}</td>", meta.not_run(app)),
            }
            body.push_str("</tr>\n");
        }
    }
    body.push_str("</table>\n");
    body.push_str(
        "<div class=\"toolbar\" data-js hidden><button type=\"button\" data-open=\"1\">Expand all</button>\
         <button type=\"button\" data-open=\"0\">Collapse all</button></div>\n",
    );

    for size in &data.sizes {
        w!(body, "<h2>{}</h2>\n", escape(&size.summary.size));
        if size.captures.is_empty() {
            body.push_str("<p class=\"muted\">This scenario has no captures.</p>\n");
        }
        for capture in &size.captures {
            body.push_str(&render_capture(size, capture, meta, &mut sheet));
        }
        for app in App::ALL {
            if let Some(log) = size.logs.get(app) {
                w!(
                    body,
                    "<details><summary>fake-gh.log ({app}, last {LOG_TAIL_LINES} lines)</summary>\
                     <pre class=\"log\">{}</pre></details>\n",
                    escape(log)
                );
            }
        }
    }

    page(&format!("Parity: {}", s.name), &sheet.css(), &body)
}

fn status_cells(status: &AppRunStatus) -> String {
    let mut word = state_name(status.status).to_owned();
    if status.fixture_problems {
        word.push_str(", fixture problems");
    }
    let mut cell = escape(&word);
    if let Some(message) = &status.message {
        w!(cell, "<br><span class=\"muted\">{}</span>", escape(message));
    }
    let opt = |v: Option<String>| v.map_or_else(|| "<span class=\"muted\">-</span>".to_owned(), |v| escape(&v));
    let mut misses = String::new();
    for miss in &status.misses {
        w!(misses, "<code>{}</code><br>", escape(&miss_key(miss)));
    }
    for error in &status.errors {
        w!(
            misses,
            "<span class=\"muted\">error:</span> {}<br>",
            escape(error.error.as_deref().unwrap_or_default())
        );
    }
    if misses.is_empty() {
        misses.push_str("<span class=\"muted\">none</span>");
    }
    // A recorded side took no time: say where it came from instead.
    let duration = if status.recorded {
        "recorded".to_owned()
    } else {
        format!("{:.1} s", Duration::from_millis(status.duration_ms).as_secs_f64())
    };
    format!(
        "<td>{cell}</td><td>{duration}</td><td>{}</td><td>{}</td><td>{}</td><td>{misses}</td>",
        if status.flaky { "yes" } else { "no" },
        opt(status.step.map(|s| s.to_string())),
        opt(status.exit_code.map(|c| c.to_string())),
    )
}

/// The structural mismatch table of one capture, or why there isn't one.
fn render_mismatches(check: &CheckState) -> String {
    let mut out = String::new();
    match check {
        CheckState::Absent => out.push_str("<p class=\"muted\">not checked</p>\n"),
        CheckState::Unreadable(e) => w!(
            out,
            "<p class=\"reason\">check.json couldn't be read: {}</p>\n",
            escape(e)
        ),
        CheckState::Loaded(check) if check.len() == 0 => out.push_str("<p>No mismatches.</p>\n"),
        CheckState::Loaded(check) => {
            out.push_str(
                "<table>\n<tr><th>Check</th><th>Box</th><th>Python</th><th>Rust</th><th>Expected</th>\
                 <th>Message</th></tr>\n",
            );
            for (kind, m) in check.mismatches() {
                let expected = if m.expected.is_null() {
                    String::new()
                } else {
                    format!("<code>{}</code>", escape(&value_text(&m.expected)))
                };
                w!(
                    out,
                    "<tr><td>{} <span class=\"muted\">({kind})</span></td><td>{}</td><td><code>{}</code></td>\
                     <td><code>{}</code></td><td>{expected}</td><td>{}</td></tr>\n",
                    escape(&m.check),
                    escape(&value_text(&m.box_title)),
                    escape(&mismatch_value(&m.python)),
                    escape(&mismatch_value(&m.rust)),
                    escape(&m.message)
                );
            }
            out.push_str("</table>\n");
        }
    }
    out
}

fn render_capture(size: &SizeData<'_>, capture: &CaptureData, meta: &RunMeta, sheet: &mut StyleSheet) -> String {
    let mut html = String::new();
    w!(
        html,
        "<section class=\"capture\" id=\"{}\">\n<h3>{}</h3>\n",
        anchor(&size.summary.size, &capture.name),
        escape(&capture.name)
    );

    let both = capture.grids.python.as_ref().zip(capture.grids.rust.as_ref());
    let mut panels = String::from("<div class=\"panels\">\n");
    let mut differing = 0;
    for app in App::ALL {
        let recorded = size.summary.apps.get(&app).is_some_and(|s| s.recorded);
        w!(
            panels,
            "<figure class=\"panel\"><figcaption>{}{}</figcaption>",
            label(app),
            if recorded { " (recorded)" } else { "" }
        );
        let other = both.map(|(py, rs)| if app == App::Python { rs } else { py });
        match (capture.grids.get(app), size.summary.apps.get(&app)) {
            (Some(grid), _) => {
                let (pre, n) = render_grid(grid, other, sheet);
                differing = differing.max(n);
                w!(panels, "<div class=\"scroll\">{pre}</div>");
            }
            (None, None) => w!(panels, "<div class=\"notrun\">{}</div>", meta.not_run(app)),
            (None, Some(status)) => {
                let reason = if status.passed() {
                    format!("No capture {:?}.", capture.name)
                } else {
                    format!("No capture: {}.", status.describe())
                };
                w!(panels, "<p class=\"reason\">{}</p>", escape(&reason));
                match size.failures.get(app) {
                    Some(grid) => {
                        let (pre, _) = render_grid(grid, None, sheet);
                        w!(
                            panels,
                            "<p class=\"muted\">The screen when the run failed:</p><div class=\"scroll\">{pre}</div>"
                        );
                    }
                    None => panels.push_str("<div class=\"notrun\">No screen was dumped.</div>"),
                }
            }
        }
        panels.push_str("</figure>\n");
    }
    panels.push_str("</div>\n");

    if both.is_some() {
        w!(
            html,
            "<div class=\"toolbar\"><label data-js hidden><input type=\"checkbox\" data-hl> Highlight differences</label>\
             <span class=\"muted\">{differing} cell(s) differ</span></div>\n"
        );
    }
    html.push_str(&panels);

    html.push_str("<h4>Structural mismatches</h4>\n");
    html.push_str(&render_mismatches(&capture.check));

    let diff = match &capture.diff {
        Some(d) if d.is_empty() => "<p class=\"muted\">The two captures' text is identical.</p>".to_owned(),
        Some(d) => format!("<pre class=\"log\">{}</pre>", escape(d)),
        None => "<p class=\"muted\">Needs a capture from both apps.</p>".to_owned(),
    };
    w!(html, "<details><summary>Text diff</summary>{diff}</details>\n");
    for app in App::ALL {
        let body = capture.structures.get(app).as_ref().map_or_else(
            || "<p class=\"muted\">No structure.json.</p>".to_owned(),
            |s| format!("<pre class=\"log\">{}</pre>", escape(s)),
        );
        w!(
            html,
            "<details><summary>structure.json ({app})</summary>{body}</details>\n"
        );
    }
    html.push_str("</section>\n");
    html
}

fn label(app: App) -> &'static str {
    match app {
        App::Python => "Python",
        App::Rust => "Rust",
    }
}

/// Scenario order for the index and `summary.md`: failed gated scenarios first, then other failures, then
/// `rust-not-ready`, then passes. Ties keep run order.
fn sorted<'s, 'a>(scenarios: &'s [ScenarioData<'a>]) -> Vec<&'s ScenarioData<'a>> {
    let rank = |s: &ScenarioSummary| match s.result {
        ScenarioResult::Fail if s.gate => 0,
        ScenarioResult::Fail => 1,
        ScenarioResult::RustNotReady => 2,
        ScenarioResult::PythonOnly | ScenarioResult::RustOnly | ScenarioResult::Pass => 3,
    };
    let mut out: Vec<_> = scenarios.iter().collect();
    out.sort_by_key(|s| rank(s.summary));
    out
}

/// `1 pass, 0 fail, 2 rust-not-ready`, plus the limited results when there are any.
fn counts(scenarios: &[ScenarioData<'_>]) -> String {
    let n = |r: ScenarioResult| scenarios.iter().filter(|s| s.summary.result == r).count();
    let mut out = format!(
        "{} pass, {} fail, {} rust-not-ready",
        n(ScenarioResult::Pass),
        n(ScenarioResult::Fail),
        n(ScenarioResult::RustNotReady)
    );
    for r in [ScenarioResult::PythonOnly, ScenarioResult::RustOnly] {
        if n(r) > 0 {
            w!(out, ", {} {r}", n(r));
        }
    }
    out
}

fn render_index(scenarios: &[ScenarioData<'_>], meta: &RunMeta) -> String {
    let mut body = String::new();
    w!(body, "<h1>Parity report</h1>\n<p>{}</p>\n", escape(&counts(scenarios)));
    w!(
        body,
        "<table>\n<tr><th>Commit</th><td><code>{}</code></td></tr>\n<tr><th>Time</th><td>{}</td></tr>\n\
         <tr><th>OS</th><td>{}</td></tr>\n<tr><th>Command</th><td><code>cargo xtask {}</code></td></tr>\n</table>\n\
         <p>{}</p>\n",
        escape(&meta.git_sha),
        escape(&meta.timestamp),
        escape(&meta.os),
        escape(&meta.command),
        escape(&meta.python_line())
    );
    body.push_str(
        "<table>\n<tr><th>Scenario</th><th>Gate</th><th>Size</th><th>Result</th><th>Python</th><th>Rust</th>\
         <th>Mismatches</th></tr>\n",
    );
    for data in sorted(scenarios) {
        let s = data.summary;
        let link = format!("{}/report.html", escape(&s.name));
        let gate = if s.gate { "yes" } else { "no" };
        if data.sizes.is_empty() {
            w!(
                body,
                "<tr><td><a href=\"{link}\">{}</a></td><td>{gate}</td><td>-</td><td>{}</td><td colspan=\"3\">{}</td></tr>\n",
                escape(&s.name),
                badge(s.result.as_str()),
                escape(s.error.as_deref().unwrap_or("no sizes ran"))
            );
        }
        for size in &data.sizes {
            let app_cell = |app: App| {
                size.summary.apps.get(&app).map_or_else(
                    || format!("<span class=\"muted\">{}</span>", meta.not_run(app)),
                    |st| escape(&st.describe()),
                )
            };
            let mismatches = size.mismatches().map_or_else(
                || "<span class=\"muted\">not checked</span>".to_owned(),
                |n| n.to_string(),
            );
            w!(
                body,
                "<tr><td><a href=\"{link}\">{}</a></td><td>{gate}</td><td><a href=\"{link}#{}\">{}</a></td><td>{}</td>\
                 <td>{}</td><td>{}</td><td>{mismatches}</td></tr>\n",
                escape(&s.name),
                size.captures
                    .first()
                    .map_or_else(String::new, |c| anchor(&size.summary.size, &c.name)),
                escape(&size.summary.size),
                badge(size.result(s).as_str()),
                app_cell(App::Python),
                app_cell(App::Rust),
            );
        }
    }
    body.push_str("</table>\n");
    page("Parity report", "", &body)
}

/// Escapes text for a Markdown table cell or list item: no HTML, no column breaks, one line.
fn md_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

fn render_summary_md(scenarios: &[ScenarioData<'_>], meta: &RunMeta) -> String {
    const CLOSE: &str = "\n</details>\n";

    let mut sizes: Vec<&str> = SUMMARY_SIZES.to_vec();
    for size in scenarios.iter().flat_map(|s| &s.sizes) {
        if !sizes.contains(&size.summary.size.as_str()) {
            sizes.push(&size.summary.size);
        }
    }

    let mut md = format!(
        "## Parity: {}\n\n{}\n\n| Scenario | Gate |",
        counts(scenarios),
        md_escape(&meta.python_line())
    );
    for size in &sizes {
        w!(md, " {size} |");
    }
    md.push_str(" Mismatches |\n| --- | --- |");
    md.push_str(&" --- |".repeat(sizes.len() + 1));
    md.push('\n');

    // The table stops 8 KiB short of the limit; the mismatch list below checks the limit before every line.
    let budget = SUMMARY_MD_LIMIT - 8 * 1024;
    let ordered = sorted(scenarios);
    for (i, data) in ordered.iter().enumerate() {
        let s = data.summary;
        let mut row = format!("| {} | {} |", md_escape(&s.name), if s.gate { "yes" } else { "no" });
        for size in &sizes {
            match data.sizes.iter().find(|d| d.summary.size == *size) {
                None => row.push_str(" skipped |"),
                // A run that passed only on retry doesn't change the result, but it's worth a look.
                Some(d) if d.summary.apps.values().any(|a| a.flaky) => w!(row, " {} (flaky) |", d.result(s)),
                Some(d) => w!(row, " {} |", d.result(s)),
            }
        }
        let mismatches = data.mismatches().map_or_else(|| "n/a".to_owned(), |n| n.to_string());
        w!(row, " {mismatches} |\n");
        if md.len() + row.len() > budget {
            w!(md, "\n…and {} more scenario(s); see index.html.\n", ordered.len() - i);
            break;
        }
        md.push_str(&row);
    }

    let mismatches: Vec<String> = scenarios
        .iter()
        .flat_map(|data| {
            data.sizes.iter().flat_map(move |size| {
                size.captures.iter().flat_map(move |capture| {
                    let check = match &capture.check {
                        CheckState::Loaded(c) => Some(c),
                        _ => None,
                    };
                    check.into_iter().flat_map(CheckFile::mismatches).map(move |(kind, m)| {
                        let at = format!("{}/{}/{}", data.summary.name, size.summary.size, capture.name);
                        let box_title = value_text(&m.box_title);
                        // Drift is a Python problem, unlike the other mismatches here: say so.
                        let check = if kind == "drift" {
                            format!("drift {}", m.check)
                        } else {
                            m.check.clone()
                        };
                        let target = if box_title.is_empty() {
                            check
                        } else {
                            format!("{check} {box_title}")
                        };
                        let value = |v: &Value| md_escape(&cut(&mismatch_value(v), SUMMARY_VALUE_CHARS));
                        let expected = if m.expected.is_null() {
                            String::new()
                        } else {
                            format!(" expected={}", value(&m.expected))
                        };
                        format!(
                            "- {}: {}: python={} rust={}{expected}",
                            md_escape(&cut(&at, SUMMARY_TARGET_CHARS)),
                            md_escape(&cut(&target, SUMMARY_TARGET_CHARS)),
                            value(&m.python),
                            value(&m.rust)
                        )
                    })
                })
            })
        })
        .collect();
    if !mismatches.is_empty() {
        let shown = mismatches.len().min(SUMMARY_MISMATCHES);
        w!(
            md,
            "\n<details><summary>Mismatches (first {shown} of {})</summary>\n\n",
            mismatches.len()
        );
        for line in &mismatches[..shown] {
            if md.len() + line.len() + 1 + MORE_MISMATCHES.len() + CLOSE.len() > SUMMARY_MD_LIMIT {
                md.push_str(MORE_MISMATCHES);
                break;
            }
            md.push_str(line);
            md.push('\n');
        }
        md.push_str(CLOSE);
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::grid::{Cursor, Line, Run};
    use crate::parity::summary::CaptureSummary;
    use std::collections::BTreeMap;

    fn meta() -> RunMeta {
        RunMeta {
            git_sha: "0123abcd".into(),
            subject: "Add a thing".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            os: "linux (x86_64)".into(),
            command: "parity --all".into(),
            python: AppSource::Recorded,
            rust: AppSource::Live,
        }
    }

    fn status(state: RunState) -> AppRunStatus {
        AppRunStatus {
            status: state,
            step: (state != RunState::Ok).then_some(1),
            exit_code: None,
            duration_ms: 1200,
            flaky: false,
            fixture_problems: false,
            misses: Vec::new(),
            errors: Vec::new(),
            message: None,
            recorded: false,
        }
    }

    fn size_summary(size: &str, python: RunState, rust: Option<RunState>) -> SizeSummary {
        let mut apps = BTreeMap::from([(App::Python, status(python))]);
        if let Some(rust) = rust {
            apps.insert(App::Rust, status(rust));
        }
        SizeSummary {
            size: size.into(),
            apps,
            captures: vec![CaptureSummary::unchecked("main")],
        }
    }

    fn scenario(name: &str, gate: bool, result: ScenarioResult, sizes: Vec<SizeSummary>) -> ScenarioSummary {
        ScenarioSummary {
            name: name.into(),
            description: format!("The {name} scenario"),
            gate,
            parity_rows: vec!["main-layout.layout".into()],
            result,
            sizes,
            error: None,
        }
    }

    fn capture(check: CheckState) -> CaptureData {
        CaptureData {
            name: "main".into(),
            grids: Pair::default(),
            structures: Pair::default(),
            check,
            diff: None,
        }
    }

    fn data(summary: &ScenarioSummary, checks: Vec<CheckState>) -> ScenarioData<'_> {
        let mut checks = checks.into_iter();
        ScenarioData {
            summary,
            sizes: summary
                .sizes
                .iter()
                .map(|size| SizeData {
                    summary: size,
                    captures: vec![capture(checks.next().unwrap_or(CheckState::Absent))],
                    failures: Pair::default(),
                    logs: Pair::default(),
                })
                .collect(),
        }
    }

    fn mismatch(check: &str, title: &str, python: Value, rust: Value) -> Mismatch {
        Mismatch {
            check: check.into(),
            box_title: Value::String(title.into()),
            python,
            rust,
            expected: Value::Null,
            message: format!("{check} differ"),
        }
    }

    /// A grid from text rows, every cell in `style` unless `styled` says otherwise for (row, col).
    fn grid(rows: &[&str], styled: &[(usize, usize, Style)]) -> Grid {
        let lines = rows
            .iter()
            .enumerate()
            .map(|(r, text)| {
                let cells: Vec<String> = text.chars().map(String::from).collect();
                let mut runs: Vec<Run> = Vec::new();
                for col in 0..cells.len() {
                    let style = styled
                        .iter()
                        .find(|(sr, sc, _)| *sr == r && *sc == col)
                        .map_or_else(Style::default, |(_, _, s)| s.clone());
                    let col = u16::try_from(col).unwrap();
                    match runs.last_mut() {
                        Some(run) if run.style == style => run.len += 1,
                        _ => runs.push(Run { col, len: 1, style }),
                    }
                }
                Line { cells, runs }
            })
            .collect::<Vec<_>>();
        Grid {
            version: 1,
            cols: u16::try_from(rows[0].chars().count()).unwrap(),
            rows: u16::try_from(rows.len()).unwrap(),
            cursor: Cursor {
                row: 0,
                col: 0,
                visible: false,
            },
            lines,
        }
    }

    #[test]
    fn summary_md_for_a_mixed_run() {
        use RunState::{Ok, Timeout};
        let mut pass = scenario(
            "startup",
            true,
            ScenarioResult::Pass,
            vec![
                size_summary("160x48", Ok, Some(Ok)),
                size_summary("100x30", Ok, Some(Ok)),
            ],
        );
        // Passed on retry at 100x30.
        pass.sizes[1].apps.get_mut(&App::Rust).unwrap().flaky = true;
        let fail = scenario(
            "repo-select",
            true,
            ScenarioResult::Fail,
            vec![size_summary("160x48", Ok, Some(Ok))],
        );
        let not_ready = scenario(
            "pr-list",
            false,
            ScenarioResult::RustNotReady,
            vec![
                size_summary("160x48", Ok, Some(Timeout)),
                size_summary("100x30", Ok, Some(Timeout)),
            ],
        );
        let long = "x".repeat(120);
        let checked = CheckState::Loaded(CheckFile {
            expect: Vec::new(),
            cross: vec![
                mismatch(
                    "titles",
                    "[1] Repositories",
                    Value::String("[1] Repositories".into()),
                    Value::Null,
                ),
                mismatch(
                    "rows",
                    "Pull | Requests",
                    serde_json::json!(["a", "<b>"]),
                    Value::String(long),
                ),
            ],
            // `repo-select` ran Python live, and it disagrees with its reference.
            drift: vec![mismatch(
                "rows",
                "[1] Repositories",
                serde_json::json!(["parity-bot", "alpha"]),
                serde_json::json!(["parity-bot", "beta"]),
            )],
        });
        let clean = || CheckState::Loaded(CheckFile::default());
        let scenarios = [
            data(&pass, vec![clean(), clean()]),
            data(&fail, vec![checked]),
            data(&not_ready, vec![]),
        ];
        let meta = RunMeta {
            python: AppSource::LiveFor(vec!["repo-select".into()]),
            ..meta()
        };
        let md = render_summary_md(&scenarios, &meta);
        assert!(md.len() < SUMMARY_MD_LIMIT);
        insta::assert_snapshot!(md);
    }

    #[test]
    fn summary_md_never_exceeds_the_limit() {
        // Thousands of scenarios with long names and mismatches whose every part is huge and needs escaping.
        let huge = "<&|".repeat(2_000);
        let names: Vec<String> = (0..3000).map(|i| format!("scenario-{i}-{}", "n".repeat(100))).collect();
        let summaries: Vec<_> = names
            .iter()
            .map(|n| {
                scenario(
                    n,
                    true,
                    ScenarioResult::Fail,
                    vec![size_summary("160x48", RunState::Ok, None)],
                )
            })
            .collect();
        let big = || {
            CheckState::Loaded(CheckFile {
                expect: Vec::new(),
                cross: (0..30)
                    .map(|_| Mismatch {
                        check: huge.clone(),
                        box_title: serde_json::json!({ "title": huge }),
                        python: Value::String(huge.clone()),
                        rust: serde_json::json!([huge]),
                        expected: Value::String(huge.clone()),
                        message: huge.clone(),
                    })
                    .collect(),
                drift: Vec::new(),
            })
        };
        // Two scenarios' worth of mismatches is already more than the list shows.
        let scenarios: Vec<_> = summaries
            .iter()
            .enumerate()
            .map(|(i, s)| data(s, vec![if i < 2 { big() } else { CheckState::Absent }]))
            .collect();
        let md = render_summary_md(&scenarios, &meta());
        assert!(md.len() <= SUMMARY_MD_LIMIT, "{} bytes", md.len());
        assert!(md.contains("more scenario(s); see index.html"));
        assert!(md.contains(MORE_MISMATCHES), "the mismatch list was cut too");
        assert!(md.trim_end().ends_with("</details>"));
    }

    #[test]
    fn unreadable_check_and_structure_files_dont_fail_the_report() {
        let out = tempfile::tempdir().unwrap();
        let summary = scenario(
            "startup",
            false,
            ScenarioResult::PythonOnly,
            vec![size_summary("160x48", RunState::Ok, None)],
        );
        let dir = out.path().join("startup/160x48");
        std::fs::create_dir_all(dir.join("python/main.structure.json")).unwrap(); // a directory: reading it fails
        std::fs::write(dir.join("main.check.json"), "{\"cross\": [ not json").unwrap();

        let loaded = load_scenario(out.path(), &summary).unwrap();
        let capture = &loaded.sizes[0].captures[0];
        assert!(
            matches!(&capture.check, CheckState::Unreadable(_)),
            "{:?}",
            capture.check
        );
        let structure = capture.structures.python.as_deref().unwrap();
        assert!(
            structure.starts_with("structure.json couldn't be read: "),
            "{structure}"
        );

        let html = render_scenario(&loaded, &meta());
        assert!(
            html.contains("<p class=\"reason\">check.json couldn't be read: "),
            "{html}"
        );
        assert!(html.contains("structure.json couldn&#39;t be read: "), "{html}");

        std::fs::remove_file(dir.join("main.check.json")).unwrap();
        std::fs::create_dir(dir.join("main.check.json")).unwrap();
        let loaded = load_scenario(out.path(), &summary).unwrap();
        assert!(matches!(&loaded.sizes[0].captures[0].check, CheckState::Unreadable(_)));
    }

    #[test]
    fn summary_md_marks_sizes_that_didnt_run_as_skipped() {
        let only = scenario(
            "narrow",
            false,
            ScenarioResult::PythonOnly,
            vec![size_summary("100x30", RunState::Ok, None)],
        );
        let md = render_summary_md(&[data(&only, vec![])], &meta());
        assert!(
            md.starts_with("## Parity: 0 pass, 0 fail, 0 rust-not-ready, 1 python-only\n"),
            "{md}"
        );
        assert!(md.contains("| narrow | no | skipped | python-only | n/a |"), "{md}");
    }

    #[test]
    fn html_for_a_small_grid_pair() {
        let green = Style {
            fg: Some("#4ebf71".into()),
            bold: true,
            ..Style::default()
        };
        let inverse = Style {
            inverse: true,
            ..Style::default()
        };
        let indexed = Style {
            fg: Some("idx:196".into()),
            bg: Some("idx:4".into()),
            ..Style::default()
        };
        let py = grid(
            &["<a&\"", "ok  "],
            &[(0, 0, green.clone()), (0, 1, green.clone()), (1, 0, inverse.clone())],
        );
        let rs = grid(
            &["<b&\"", "ok  "],
            &[(0, 0, green.clone()), (0, 1, green), (1, 0, inverse), (1, 3, indexed)],
        );
        let mut sheet = StyleSheet::default();
        let (py_html, py_diff) = render_grid(&py, Some(&rs), &mut sheet);
        let (rs_html, rs_diff) = render_grid(&rs, Some(&py), &mut sheet);
        assert_eq!((py_diff, rs_diff), (1, 1));
        insta::assert_snapshot!(format!("{}\n{py_html}\n{rs_html}", sheet.css()));
    }

    #[test]
    fn a_full_screen_panel_stays_small() {
        // Worst case for span merging: every cell's style differs from its neighbor's.
        let a = Style {
            fg: Some("#4ebf71".into()),
            bg: Some("#121212".into()),
            bold: true,
            ..Style::default()
        };
        let b = Style {
            fg: Some("#e0e0e0".into()),
            bg: Some("#2d3740".into()),
            underline: true,
            ..Style::default()
        };
        let row = "─".repeat(160);
        let rows = vec![row.as_str(); 48];
        let styled: Vec<_> = (0..48)
            .flat_map(|r| (0..160).map(move |c| (r, c)))
            .map(|(r, c)| (r, c, if (r + c) % 2 == 0 { a.clone() } else { b.clone() }))
            .collect();
        let g = grid(&rows, &styled);
        let (html, _) = render_grid(&g, Some(&grid(&rows, &[])), &mut StyleSheet::default());
        assert!(html.len() < 300 * 1024, "{} bytes", html.len());
    }

    #[test]
    fn colors_are_validated_and_inverse_swaps() {
        assert_eq!(css_color("#ABCDEF").as_deref(), Some("#abcdef"));
        assert_eq!(css_color("red;}body{x:y").as_deref(), None);
        assert_eq!(css_color("#12345").as_deref(), None);
        assert_eq!(css_color("idx:1").as_deref(), Some("#cd0000"));
        assert_eq!(css_color("idx:16").as_deref(), Some("#000000"));
        assert_eq!(css_color("idx:231").as_deref(), Some("#ffffff"));
        assert_eq!(css_color("idx:255").as_deref(), Some("#eeeeee"));
        assert_eq!(css_color("idx:256"), None);
        let inverse = Style {
            fg: Some("#ff0000".into()),
            inverse: true,
            ..Style::default()
        };
        assert_eq!(style_css(&inverse), format!("color:{DEFAULT_BG};background:#ff0000;"));
        assert_eq!(style_css(&Style::default()), "");
    }

    #[test]
    fn pages_reference_no_external_assets() {
        use RunState::{Exited, Ok};
        let summary = scenario(
            "startup",
            false,
            ScenarioResult::RustNotReady,
            vec![size_summary("160x48", Ok, Some(Exited))],
        );
        let mut d = data(
            &summary,
            vec![CheckState::Loaded(CheckFile {
                expect: vec![mismatch("titles", "Repos", Value::Null, Value::Null)],
                cross: Vec::new(),
                drift: Vec::new(),
            })],
        );
        let screen = grid(&["┌─ [1] Repositories ─┐", "└────────────────────┘"], &[]);
        d.sizes[0].captures[0].grids.python = Some(screen.clone());
        d.sizes[0].captures[0].structures.python = Some("{\"version\": 1}".into());
        d.sizes[0].failures.rust = Some(screen);
        d.sizes[0].logs.python = Some("{\"seq\":1}".into());
        let pages = [render_scenario(&d, &meta()), render_index(&[d.clone()], &meta())];
        for html in &pages {
            for needle in ["http://", "https://", "<link", " src=", "@import", "url("] {
                assert!(!html.contains(needle), "found {needle:?}");
            }
            // The JS stays under 100 lines.
            assert!(PAGE_JS.lines().count() < 100);
        }
        assert!(
            pages[0].contains("No capture: exited at step 1 (1.2s)."),
            "{}",
            pages[0]
        );
        assert!(pages[0].contains("The screen when the run failed"));
    }

    #[test]
    fn a_recorded_python_side_says_so() {
        let mut summary = scenario(
            "startup",
            true,
            ScenarioResult::Pass,
            vec![size_summary("160x48", RunState::Ok, Some(RunState::Ok))],
        );
        summary.sizes[0].apps.insert(App::Python, AppRunStatus::recorded());
        let mut d = data(&summary, vec![CheckState::Loaded(CheckFile::default())]);
        d.sizes[0].captures[0].grids.python = Some(grid(&["hi"], &[]));
        d.sizes[0].captures[0].grids.rust = Some(grid(&["hi"], &[]));
        let html = render_scenario(&d, &meta());
        let section = |start: &str, end: &str| {
            let from = html.find(start).unwrap();
            let len = html[from..].find(end).unwrap() + end.len();
            html[from..from + len].to_owned()
        };
        let runs = section("<table>\n<tr><th>Size</th>", "</table>\n");
        let panels = section("<div class=\"panels\">", "</figure>\n</div>\n");
        insta::assert_snapshot!(format!("{runs}{panels}"));

        let index = render_index(&[d.clone()], &meta());
        assert!(index.contains("<p>Python: recorded</p>"), "{index}");
        assert!(index.contains("<td>recorded</td>"), "{index}");
    }

    #[test]
    fn app_sources_read_as_the_report_says_them() {
        assert_eq!(AppSource::NotRun.to_string(), "not run");
        assert_eq!(AppSource::Recorded.to_string(), "recorded");
        assert_eq!(AppSource::Live.to_string(), "live");
        assert_eq!(
            AppSource::LiveFor(vec!["a".into(), "b".into()]).to_string(),
            "live for a, b; recorded for the rest"
        );
    }

    #[test]
    fn missing_apps_say_why() {
        let summary = scenario(
            "startup",
            false,
            ScenarioResult::PythonOnly,
            vec![size_summary("160x48", RunState::Ok, None)],
        );
        let mut d = data(&summary, vec![]);
        d.sizes[0].captures[0].grids.python = Some(grid(&["hi"], &[]));
        let meta = RunMeta {
            rust: AppSource::NotRun,
            ..meta()
        };
        let html = render_scenario(&d, &meta);
        assert!(html.contains("<div class=\"notrun\">not run (--python-only)</div>"));
        assert!(html.contains("<p class=\"muted\">not checked</p>"));
        assert!(!html.contains("Highlight differences"), "no toggle with one panel");
    }

    #[test]
    fn index_sorts_failures_first() {
        let pass = scenario("a-pass", true, ScenarioResult::Pass, vec![]);
        let not_ready = scenario("b-not-ready", false, ScenarioResult::RustNotReady, vec![]);
        let fail = scenario("c-fail", true, ScenarioResult::Fail, vec![]);
        let scenarios = [data(&pass, vec![]), data(&not_ready, vec![]), data(&fail, vec![])];
        let order: Vec<_> = sorted(&scenarios).iter().map(|d| d.summary.name.as_str()).collect();
        assert_eq!(order, ["c-fail", "b-not-ready", "a-pass"]);
    }

    #[test]
    fn miss_keys_include_the_query() {
        let entry: LogEntry = serde_json::from_value(serde_json::json!({
            "seq": 1, "argv": [], "command": "api", "method": "GET", "path": "/user/repos",
            "query": {"page": "1", "q": "a b"}, "accept": null, "cache": null, "cache_control": null, "body": null,
            "fixture": null, "status": 404, "miss": true, "error": null
        }))
        .unwrap();
        assert_eq!(miss_key(&entry), "GET /user/repos?page=1&q=a+b");
    }
}
