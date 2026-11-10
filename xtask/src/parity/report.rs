//! The human-readable report, written at the end of every run from `summary.json` and the dumps on disk: a
//! self-contained `report.html` per scenario, `index.html` and `summary.md`.
//!
//! Each capture shows the run's screen. When it differs from its committed snapshot, the snapshot (the `.screen` the
//! check copied next to the run) and the run's masked screen are shown side by side, with the differing cells
//! marked. `check.json` and `structure.json` are read when they exist; without them the report says "not checked".
//! Everything here is plain string building: the markup is small and fixed, so there's no template engine.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use fake_gh::log::LogEntry;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::env::BROWSER_LOG;
use super::grid::{Grid, Style};
use super::runner::{AppRunStatus, RunState};
use super::screen;
use super::snapshot::{NEW_SUFFIX, SCREEN_EXTENSION, SNAPSHOT_SUFFIX};
use super::summary::{ScenarioResult, ScenarioSummary, SizeSummary, Summary};
use super::{Context, Result};

/// The terminal's default foreground, for cells whose fg is `null`.
pub const DEFAULT_FG: &str = "#d4d4d4";
/// The terminal's default background, for cells whose bg is `null`.
pub const DEFAULT_BG: &str = "#1e1e1e";
/// How much of the run's `fake-gh.log` the report shows.
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
    /// The command line after `cargo xtask`, e.g. `parity startup --ci`.
    pub command: String,
}

impl RunMeta {
    /// The current run: `git rev-parse HEAD` in `repo`, the time now and this process's arguments.
    pub fn current(repo: &Path) -> Self {
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
        }
    }

    /// Reads the [`META_FILE`] a run wrote into the report directory `dir`.
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join(META_FILE);
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }
}

/// Writes [`META_FILE`], each scenario's `report.html`, `index.html` and `summary.md` into `out`.
pub fn write_reports(out: &Path, summary: &Summary, meta: &RunMeta) -> Result<()> {
    // Plain strings; serializing can't fail.
    let json = serde_json::to_string_pretty(meta).expect("run metadata serializes");
    std::fs::write(out.join(META_FILE), json + "\n").with_context(|| format!("writing {META_FILE}"))?;
    let mut scenarios = Vec::with_capacity(summary.scenarios.len());
    for scenario in &summary.scenarios {
        let data = load_scenario(out, scenario)?;
        let dir = out.join(&scenario.name);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        std::fs::write(dir.join("report.html"), render_scenario(&data))
            .with_context(|| format!("writing {}/report.html", scenario.name))?;
        scenarios.push(data);
    }
    std::fs::write(out.join("index.html"), render_index(&scenarios, meta)).context("writing index.html")?;
    std::fs::write(out.join("summary.md"), render_summary_md(&scenarios)).context("writing summary.md")?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Loading

/// One mismatch from `check.json`. Every field is optional so a report never fails on a newer or partial file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Mismatch {
    check: String,
    #[serde(rename = "box")]
    box_title: Value,
    /// What the screen showed; `null` for a snapshot mismatch.
    actual: Value,
    /// What `[expect]` wanted; `null` for a snapshot mismatch.
    expected: Value,
    message: String,
}

/// `<capture>.check.json`: `expect` mismatches are the screen against `[expect]`, `snapshot` ones the run against
/// its committed snapshot.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct CheckFile {
    expect: Vec<Mismatch>,
    snapshot: Vec<Mismatch>,
}

impl CheckFile {
    fn mismatches(&self) -> impl Iterator<Item = (&'static str, &Mismatch)> {
        let expect = self.expect.iter().map(|m| ("expect", m));
        expect.chain(self.snapshot.iter().map(|m| ("snapshot", m)))
    }

    fn len(&self) -> usize {
        self.expect.len() + self.snapshot.len()
    }
}

/// The diff a snapshot mismatch links to, relative to the scenario's `report.html`: the capture's
/// `<capture>.snapshot.diff.txt`, or `effects.diff.txt`, in the run's directory.
fn snapshot_diff_link(size: &str, capture: &str, m: &Mismatch) -> String {
    if m.check == "effects" {
        format!("{size}/effects.diff.txt")
    } else {
        format!("{size}/{capture}.snapshot.diff.txt")
    }
}

#[derive(Debug, Clone)]
enum CheckState {
    /// No `check.json`: nothing checked the capture.
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

/// A capture's run and, when the run differs from it, its snapshot.
#[derive(Debug, Clone, Default)]
struct Differs {
    /// The committed `.screen` (`<capture>.screen.snapshot`).
    snapshot: Option<Grid>,
    /// The run's masked screen (`<capture>.screen.new`), what the snapshot is compared with.
    run: Option<Grid>,
    /// `<capture>.snapshot.diff.txt`.
    diff: Option<String>,
}

#[derive(Debug, Clone)]
struct CaptureData {
    name: String,
    /// The run's screen (`<capture>.grid.json`).
    grid: Option<Grid>,
    differs: Differs,
    /// Pretty-printed `structure.json`.
    structure: Option<String>,
    check: CheckState,
}

#[derive(Debug, Clone)]
struct SizeData<'a> {
    summary: &'a SizeSummary,
    captures: Vec<CaptureData>,
    /// `failure.grid.json`.
    failure: Option<Grid>,
    /// The last [`LOG_TAIL_LINES`] lines of `fake-gh.log`.
    log: Option<String>,
    /// `browser.jsonl`, when the app opened something.
    browser: Option<String>,
}

impl SizeData<'_> {
    /// Total mismatches, or `None` when no capture at this size was checked.
    fn mismatches(&self) -> Option<usize> {
        self.captures
            .iter()
            .filter_map(|c| c.check.count())
            .reduce(|a, b| a + b)
    }

    /// This size's own result.
    fn result(&self) -> ScenarioResult {
        ScenarioResult::from_sizes(std::slice::from_ref(self.summary))
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

/// A `.screen` file, or `None` when it's missing or doesn't parse (the check already said what differs).
fn read_screen(path: &Path) -> Result<Option<Grid>> {
    Ok(read_optional(path)?.and_then(|text| screen::parse(&text).ok()))
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

/// Reads one scenario's dumps.
fn load_scenario<'a>(out: &Path, scenario: &'a ScenarioSummary) -> Result<ScenarioData<'a>> {
    let dir = out.join(&scenario.name);
    let mut sizes = Vec::with_capacity(scenario.sizes.len());
    for size in &scenario.sizes {
        let run = dir.join(&size.size);
        let mut captures = Vec::with_capacity(size.captures.len());
        for capture in &size.captures {
            let name = &capture.name;
            let screen_file = |suffix: &str| run.join(format!("{name}.{SCREEN_EXTENSION}{suffix}"));
            let differs = Differs {
                snapshot: read_screen(&screen_file(SNAPSHOT_SUFFIX))?,
                run: read_screen(&screen_file(NEW_SUFFIX))?,
                diff: read_optional(&run.join(format!("{name}.snapshot.diff.txt")))?,
            };
            // The check's files never fail the report: any read or parse problem is shown on the page instead.
            let structure = match read_optional(&run.join(format!("{name}.structure.json"))) {
                Ok(text) => text.map(|t| pretty_json(&t)),
                Err(e) => Some(format!("structure.json couldn't be read: {e}")),
            };
            let check = match read_optional(&run.join(format!("{name}.check.json"))) {
                Ok(None) => CheckState::Absent,
                Ok(Some(text)) => match serde_json::from_str::<CheckFile>(&text) {
                    Ok(check) => CheckState::Loaded(check),
                    Err(e) => CheckState::Unreadable(e.to_string()),
                },
                Err(e) => CheckState::Unreadable(e.to_string()),
            };
            captures.push(CaptureData {
                name: name.clone(),
                grid: read_grid(&run.join(format!("{name}.grid.json")))?,
                differs,
                structure,
                check,
            });
        }
        sizes.push(SizeData {
            summary: size,
            captures,
            failure: read_grid(&run.join("failure.grid.json"))?,
            log: read_optional(&run.join("fake-gh.log"))?.map(|t| tail(&t, LOG_TAIL_LINES)),
            browser: read_optional(&run.join(BROWSER_LOG))?.filter(|text| fake_gh::browser::has_opened(text)),
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
.b-pass{background:#1a7f37}.b-fail{background:#cf222e}
.panels{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px}.panels.single{grid-template-columns:minmax(0,1fr)}
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

/// A mismatch's actual or expected value. `null` (the screen had nothing there) is spelled out.
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

fn render_scenario(data: &ScenarioData<'_>) -> String {
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
        "<table>\n<tr><th>Size</th><th>Status</th><th>Duration</th><th>Flaky</th><th>Failed step</th>\
         <th>Exit code</th><th>Fixture misses</th></tr>\n",
    );
    for size in &data.sizes {
        w!(
            body,
            "<tr><td>{}</td>{}</tr>\n",
            escape(&size.summary.size),
            status_cells(&size.summary.status)
        );
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
            body.push_str(&render_capture(size, capture, &mut sheet));
        }
        if let Some(log) = &size.log {
            w!(
                body,
                "<details><summary>fake-gh.log (last {LOG_TAIL_LINES} lines)</summary>\
                 <pre class=\"log\">{}</pre></details>\n",
                escape(log)
            );
        }
        if let Some(log) = &size.browser {
            w!(
                body,
                "<details><summary>{BROWSER_LOG}: what the app opened in the browser</summary>\
                 <pre class=\"log\">{}</pre></details>\n",
                escape(log)
            );
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
    format!(
        "<td>{cell}</td><td>{:.1} s</td><td>{}</td><td>{}</td><td>{}</td><td>{misses}</td>",
        Duration::from_millis(status.duration_ms).as_secs_f64(),
        if status.flaky { "yes" } else { "no" },
        opt(status.step.map(|s| s.to_string())),
        opt(status.exit_code.map(|c| c.to_string())),
    )
}

/// The mismatch table of one capture at `size`, or why there isn't one.
fn render_mismatches(check: &CheckState, size: &str, capture: &str) -> String {
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
                "<table>\n<tr><th>Check</th><th>Box</th><th>Actual</th><th>Expected</th><th>Message</th></tr>\n",
            );
            for (kind, m) in check.mismatches() {
                // A snapshot mismatch has neither side; an `[expect]` one with nothing on screen says so.
                let code = |v: &Value| {
                    if kind == "snapshot" {
                        String::new()
                    } else {
                        format!("<code>{}</code>", escape(&mismatch_value(v)))
                    }
                };
                let link = if kind == "snapshot" {
                    format!(
                        " <a href=\"{}\">diff</a>",
                        escape(&snapshot_diff_link(size, capture, m))
                    )
                } else {
                    String::new()
                };
                w!(
                    out,
                    "<tr><td>{} <span class=\"muted\">({kind})</span></td><td>{}</td><td>{}</td><td>{}</td>\
                     <td>{}{link}</td></tr>\n",
                    escape(&m.check),
                    escape(&value_text(&m.box_title)),
                    code(&m.actual),
                    code(&m.expected),
                    escape(&m.message)
                );
            }
            out.push_str("</table>\n");
        }
    }
    out
}

/// One screen in a panel.
fn panel(caption: &str, grid: &Grid, other: Option<&Grid>, sheet: &mut StyleSheet) -> (String, usize) {
    let (pre, n) = render_grid(grid, other, sheet);
    (
        format!(
            "<figure class=\"panel\"><figcaption>{caption}</figcaption><div class=\"scroll\">{pre}</div></figure>\n"
        ),
        n,
    )
}

fn render_capture(size: &SizeData<'_>, capture: &CaptureData, sheet: &mut StyleSheet) -> String {
    let mut html = String::new();
    w!(
        html,
        "<section class=\"capture\" id=\"{}\">\n<h3>{}</h3>\n",
        anchor(&size.summary.size, &capture.name),
        escape(&capture.name)
    );

    // The run differs from its snapshot: both, masked alike, cell against cell.
    if let (Some(snapshot), Some(run)) = (&capture.differs.snapshot, &capture.differs.run) {
        let (left, a) = panel("Snapshot", snapshot, Some(run), sheet);
        let (right, b) = panel("Run", run, Some(snapshot), sheet);
        w!(
            html,
            "<div class=\"toolbar\"><label data-js hidden><input type=\"checkbox\" data-hl> Highlight differences\
             </label><span class=\"muted\">{} cell(s) differ</span></div>\n<div class=\"panels\">\n{left}{right}\
             </div>\n",
            a.max(b)
        );
    } else {
        html.push_str("<div class=\"panels single\">\n");
        match (&capture.grid, &size.failure) {
            (Some(grid), _) => html.push_str(&panel("Run", grid, None, sheet).0),
            (None, failure) => {
                let status = &size.summary.status;
                let reason = if status.passed() {
                    format!("No capture {:?}.", capture.name)
                } else {
                    format!("No capture: {}.", status.describe())
                };
                w!(
                    html,
                    "<figure class=\"panel\"><p class=\"reason\">{}</p>",
                    escape(&reason)
                );
                match failure {
                    Some(grid) => {
                        let (pre, _) = render_grid(grid, None, sheet);
                        w!(
                            html,
                            "<p class=\"muted\">The screen when the run failed:</p><div class=\"scroll\">{pre}</div>"
                        );
                    }
                    None => html.push_str("<div class=\"notrun\">No screen was dumped.</div>"),
                }
                html.push_str("</figure>\n");
            }
        }
        html.push_str("</div>\n");
    }

    html.push_str("<h4>Mismatches</h4>\n");
    html.push_str(&render_mismatches(&capture.check, &size.summary.size, &capture.name));

    if let Some(diff) = &capture.differs.diff {
        w!(
            html,
            "<details><summary>Text diff against the snapshot</summary><pre class=\"log\">{}</pre></details>\n",
            escape(diff)
        );
    }
    let structure = capture.structure.as_ref().map_or_else(
        || "<p class=\"muted\">No structure.json.</p>".to_owned(),
        |s| format!("<pre class=\"log\">{}</pre>", escape(s)),
    );
    w!(
        html,
        "<details><summary>structure.json</summary>{structure}</details>\n"
    );
    html.push_str("</section>\n");
    html
}

/// Scenario order for the index and `summary.md`: failures first, then passes. Ties keep run order.
fn sorted<'s, 'a>(scenarios: &'s [ScenarioData<'a>]) -> Vec<&'s ScenarioData<'a>> {
    let mut out: Vec<_> = scenarios.iter().collect();
    out.sort_by_key(|s| s.summary.result != ScenarioResult::Fail);
    out
}

/// `3 pass, 1 fail`.
fn counts(scenarios: &[ScenarioData<'_>]) -> String {
    let n = |r: ScenarioResult| scenarios.iter().filter(|s| s.summary.result == r).count();
    format!("{} pass, {} fail", n(ScenarioResult::Pass), n(ScenarioResult::Fail))
}

fn render_index(scenarios: &[ScenarioData<'_>], meta: &RunMeta) -> String {
    let mut body = String::new();
    w!(body, "<h1>Parity report</h1>\n<p>{}</p>\n", escape(&counts(scenarios)));
    w!(
        body,
        "<table>\n<tr><th>Commit</th><td><code>{}</code></td></tr>\n<tr><th>Time</th><td>{}</td></tr>\n\
         <tr><th>OS</th><td>{}</td></tr>\n<tr><th>Command</th><td><code>cargo xtask {}</code></td></tr>\n</table>\n",
        escape(&meta.git_sha),
        escape(&meta.timestamp),
        escape(&meta.os),
        escape(&meta.command),
    );
    body.push_str("<table>\n<tr><th>Scenario</th><th>Size</th><th>Result</th><th>Run</th><th>Mismatches</th></tr>\n");
    for data in sorted(scenarios) {
        let s = data.summary;
        let link = format!("{}/report.html", escape(&s.name));
        if data.sizes.is_empty() {
            w!(
                body,
                "<tr><td><a href=\"{link}\">{}</a></td><td>-</td><td>{}</td><td colspan=\"2\">{}</td></tr>\n",
                escape(&s.name),
                badge(s.result.as_str()),
                escape(s.error.as_deref().unwrap_or("no sizes ran"))
            );
        }
        for size in &data.sizes {
            let mismatches = size.mismatches().map_or_else(
                || "<span class=\"muted\">not checked</span>".to_owned(),
                |n| n.to_string(),
            );
            w!(
                body,
                "<tr><td><a href=\"{link}\">{}</a></td><td><a href=\"{link}#{}\">{}</a></td><td>{}</td><td>{}</td>\
                 <td>{mismatches}</td></tr>\n",
                escape(&s.name),
                size.captures
                    .first()
                    .map_or_else(String::new, |c| anchor(&size.summary.size, &c.name)),
                escape(&size.summary.size),
                badge(size.result().as_str()),
                escape(&size.summary.status.describe()),
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

fn render_summary_md(scenarios: &[ScenarioData<'_>]) -> String {
    const CLOSE: &str = "\n</details>\n";

    let mut sizes: Vec<&str> = SUMMARY_SIZES.to_vec();
    for size in scenarios.iter().flat_map(|s| &s.sizes) {
        if !sizes.contains(&size.summary.size.as_str()) {
            sizes.push(&size.summary.size);
        }
    }

    let mut md = format!("## Scenarios: {}\n\n| Scenario |", counts(scenarios));
    for size in &sizes {
        w!(md, " {size} |");
    }
    md.push_str(" Mismatches |\n| --- |");
    md.push_str(&" --- |".repeat(sizes.len() + 1));
    md.push('\n');

    // The table stops 8 KiB short of the limit; the mismatch list below checks the limit before every line.
    let budget = SUMMARY_MD_LIMIT - 8 * 1024;
    let ordered = sorted(scenarios);
    for (i, data) in ordered.iter().enumerate() {
        let mut row = format!("| {} |", md_escape(&data.summary.name));
        for size in &sizes {
            match data.sizes.iter().find(|d| d.summary.size == *size) {
                None => row.push_str(" skipped |"),
                // A run that passed only on retry doesn't change the result, but it's worth a look.
                Some(d) if d.summary.status.flaky => w!(row, " {} (flaky) |", d.result()),
                Some(d) => w!(row, " {} |", d.result()),
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
                        let target = if box_title.is_empty() {
                            m.check.clone()
                        } else {
                            format!("{} {box_title}", m.check)
                        };
                        let value = |v: &Value| md_escape(&cut(&mismatch_value(v), SUMMARY_VALUE_CHARS));
                        // A snapshot mismatch's message says what differs; an `[expect]` one shows both sides.
                        let detail = if kind == "snapshot" {
                            md_escape(&cut(&m.message, SUMMARY_TARGET_CHARS))
                        } else {
                            format!("actual={} expected={}", value(&m.actual), value(&m.expected))
                        };
                        format!(
                            "- {}: {}: {detail}",
                            md_escape(&cut(&at, SUMMARY_TARGET_CHARS)),
                            md_escape(&cut(&target, SUMMARY_TARGET_CHARS)),
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

    fn meta() -> RunMeta {
        RunMeta {
            git_sha: "0123abcd".into(),
            subject: "Add a thing".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            os: "linux (x86_64)".into(),
            command: "parity --all".into(),
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
        }
    }

    fn size_summary(size: &str, state: RunState) -> SizeSummary {
        SizeSummary {
            size: size.into(),
            status: status(state),
            captures: vec![CaptureSummary::unchecked("main")],
        }
    }

    fn scenario(name: &str, result: ScenarioResult, sizes: Vec<SizeSummary>) -> ScenarioSummary {
        ScenarioSummary {
            name: name.into(),
            description: format!("The {name} scenario"),
            parity_rows: vec!["main-layout.layout".into()],
            result,
            sizes,
            error: None,
        }
    }

    fn capture(check: CheckState) -> CaptureData {
        CaptureData {
            name: "main".into(),
            grid: None,
            differs: Differs::default(),
            structure: None,
            check,
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
                    failure: None,
                    log: None,
                    browser: None,
                })
                .collect(),
        }
    }

    fn mismatch(check: &str, title: &str, actual: Value, expected: Value) -> Mismatch {
        Mismatch {
            check: check.into(),
            box_title: Value::String(title.into()),
            actual,
            expected,
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
            ScenarioResult::Pass,
            vec![size_summary("160x48", Ok), size_summary("100x30", Ok)],
        );
        // Passed on retry at 100x30.
        pass.sizes[1].status.flaky = true;
        let mut fail = scenario("repo-select", ScenarioResult::Fail, vec![size_summary("160x48", Ok)]);
        fail.sizes[0].captures[0] = CaptureSummary {
            name: "main".into(),
            check: crate::parity::summary::CheckOutcome::Fail,
            mismatches: 3,
        };
        let timeout = scenario(
            "pr-list",
            ScenarioResult::Fail,
            vec![size_summary("160x48", Timeout), size_summary("100x30", Ok)],
        );
        let long = "x".repeat(120);
        let checked = CheckState::Loaded(CheckFile {
            expect: vec![
                mismatch(
                    "titles",
                    "[1] Repositories",
                    Value::Null,
                    Value::String("[1] Repositories".into()),
                ),
                mismatch(
                    "contains",
                    "Pull | Requests",
                    Value::String(long),
                    serde_json::json!(["a", "<b>"]),
                ),
            ],
            snapshot: vec![Mismatch {
                message: "3 line(s) differ from the snapshot".into(),
                ..mismatch("snapshot", "", Value::Null, Value::Null)
            }],
        });
        let clean = || CheckState::Loaded(CheckFile::default());
        let scenarios = [
            data(&pass, vec![clean(), clean()]),
            data(&fail, vec![checked]),
            data(&timeout, vec![]),
        ];
        let md = render_summary_md(&scenarios);
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
            .map(|n| scenario(n, ScenarioResult::Fail, vec![size_summary("160x48", RunState::Ok)]))
            .collect();
        let big = || {
            CheckState::Loaded(CheckFile {
                expect: (0..30)
                    .map(|_| Mismatch {
                        check: huge.clone(),
                        box_title: serde_json::json!({ "title": huge }),
                        actual: Value::String(huge.clone()),
                        expected: serde_json::json!([huge]),
                        message: huge.clone(),
                    })
                    .collect(),
                snapshot: Vec::new(),
            })
        };
        // Two scenarios' worth of mismatches is already more than the list shows.
        let scenarios: Vec<_> = summaries
            .iter()
            .enumerate()
            .map(|(i, s)| data(s, vec![if i < 2 { big() } else { CheckState::Absent }]))
            .collect();
        let md = render_summary_md(&scenarios);
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
            ScenarioResult::Pass,
            vec![size_summary("160x48", RunState::Ok)],
        );
        let dir = out.path().join("startup/160x48");
        std::fs::create_dir_all(dir.join("main.structure.json")).unwrap(); // a directory: reading it fails
        std::fs::write(dir.join("main.check.json"), "{\"expect\": [ not json").unwrap();

        let loaded = load_scenario(out.path(), &summary).unwrap();
        let capture = &loaded.sizes[0].captures[0];
        assert!(
            matches!(&capture.check, CheckState::Unreadable(_)),
            "{:?}",
            capture.check
        );
        let structure = capture.structure.as_deref().unwrap();
        assert!(
            structure.starts_with("structure.json couldn't be read: "),
            "{structure}"
        );

        let html = render_scenario(&loaded);
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
            ScenarioResult::Pass,
            vec![size_summary("100x30", RunState::Ok)],
        );
        let md = render_summary_md(&[data(&only, vec![])]);
        assert!(md.starts_with("## Scenarios: 1 pass, 0 fail\n"), "{md}");
        assert!(md.contains("| narrow | skipped | pass | n/a |"), "{md}");
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
        let snapshot = grid(
            &["<a&\"", "ok  "],
            &[(0, 0, green.clone()), (0, 1, green.clone()), (1, 0, inverse.clone())],
        );
        let run = grid(
            &["<b&\"", "ok  "],
            &[(0, 0, green.clone()), (0, 1, green), (1, 0, inverse), (1, 3, indexed)],
        );
        let mut sheet = StyleSheet::default();
        let (snapshot_html, a) = render_grid(&snapshot, Some(&run), &mut sheet);
        let (run_html, b) = render_grid(&run, Some(&snapshot), &mut sheet);
        assert_eq!((a, b), (1, 1));
        insta::assert_snapshot!(format!("{}\n{snapshot_html}\n{run_html}", sheet.css()));
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
        let summary = scenario(
            "startup",
            ScenarioResult::Fail,
            vec![size_summary("160x48", RunState::Exited)],
        );
        let mut d = data(
            &summary,
            vec![CheckState::Loaded(CheckFile {
                expect: vec![mismatch("titles", "Repos", Value::Null, Value::Null)],
                snapshot: vec![
                    mismatch("snapshot", "", Value::Null, Value::Null),
                    mismatch("effects", "", Value::Null, Value::Null),
                ],
            })],
        );
        let screen = grid(&["┌─ [1] Repositories ─┐", "└────────────────────┘"], &[]);
        d.sizes[0].captures[0].structure = Some("{\"version\": 1}".into());
        d.sizes[0].failure = Some(screen);
        d.sizes[0].log = Some("{\"seq\":1}".into());
        let pages = [render_scenario(&d), render_index(&[d.clone()], &meta())];
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
        // Snapshot mismatches link to their diffs, next to the run's dumps.
        assert!(pages[0].contains("<a href=\"160x48/main.snapshot.diff.txt\">diff</a>"));
        assert!(pages[0].contains("<a href=\"160x48/effects.diff.txt\">diff</a>"));
    }

    #[test]
    fn a_differing_capture_shows_its_snapshot_and_run() {
        let summary = scenario(
            "startup",
            ScenarioResult::Fail,
            vec![size_summary("160x48", RunState::Ok)],
        );
        let mut d = data(
            &summary,
            vec![CheckState::Loaded(CheckFile {
                expect: Vec::new(),
                snapshot: vec![mismatch("snapshot", "", Value::Null, Value::Null)],
            })],
        );
        let capture = &mut d.sizes[0].captures[0];
        capture.grid = Some(grid(&["hi!"], &[]));
        capture.differs = Differs {
            snapshot: Some(grid(&["hi "], &[])),
            run: Some(grid(&["hi!"], &[])),
            diff: Some("-hi\n+hi!".into()),
        };
        let html = render_scenario(&d);
        let from = html.find("<div class=\"toolbar\"><label").unwrap();
        let end = "</figure>\n</div>\n";
        let len = html[from..].find(end).unwrap() + end.len();
        insta::assert_snapshot!(&html[from..from + len]);
        assert!(html.contains("Text diff against the snapshot"));

        // Without a snapshot to compare with, only the run is shown.
        d.sizes[0].captures[0].differs = Differs::default();
        let html = render_scenario(&d);
        assert!(html.contains("<div class=\"panels single\">"));
        assert!(html.contains("<figcaption>Run</figcaption>"));
        assert!(!html.contains("<figcaption>Snapshot</figcaption>"));
        assert!(!html.contains("Highlight differences"), "no toggle with one panel");
    }

    #[test]
    fn unchecked_captures_say_so() {
        let summary = scenario(
            "startup",
            ScenarioResult::Pass,
            vec![size_summary("160x48", RunState::Ok)],
        );
        let mut d = data(&summary, vec![]);
        d.sizes[0].captures[0].grid = Some(grid(&["hi"], &[]));
        let html = render_scenario(&d);
        assert!(html.contains("<p class=\"muted\">not checked</p>"));
        let index = render_index(&[d.clone()], &meta());
        assert!(index.contains("<span class=\"muted\">not checked</span>"), "{index}");
        assert!(index.contains("<p>1 pass, 0 fail</p>"), "{index}");
    }

    #[test]
    fn index_sorts_failures_first() {
        let pass = scenario("a-pass", ScenarioResult::Pass, vec![]);
        let fail = scenario("b-fail", ScenarioResult::Fail, vec![]);
        let pass2 = scenario("c-pass", ScenarioResult::Pass, vec![]);
        let scenarios = [data(&pass, vec![]), data(&fail, vec![]), data(&pass2, vec![])];
        let order: Vec<_> = sorted(&scenarios).iter().map(|d| d.summary.name.as_str()).collect();
        assert_eq!(order, ["b-fail", "a-pass", "c-pass"]);
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
