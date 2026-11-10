//! Scenario snapshots (#396): every capture's masked screen (`<capture>.screen`, with a `<capture>.svg` drawn from it)
//! and what the run did outside the screen (`effects.txt`), committed under `parity/snapshots/`. Every run is compared
//! with them ([`check_run`]), and `cargo xtask parity --accept` writes them from two agreeing runs ([`accept`]). The
//! rules are in `parity/README.md` ("Rust snapshots").
//!
//! Masking, effects, comparison and the problems `xtask/tests/snapshots.rs` looks for are pure functions over
//! [`Grid`]s and text; only [`check_run`], [`accept`] and the orphan cleanup touch the disk.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use fake_gh::browser::BrowserLine;
use fake_gh::fixture::{Accept, Query, QueryValue};
use fake_gh::log::RequestLog;
use lazy_github_core::screen::{CellStyle, Color, Screen, SvgOptions};
use regex::Regex;
use serde_json::Value;

use super::check::{self, Mismatch};
use super::cli::{self, RunOptions};
use super::diff;
use super::env::{self, Paths};
use super::grid::{Cursor, Grid, Line, Run, Style};
use super::scenario::{Capture, Scenario, Size, Step};
use super::screen;
use super::structure::{self, Structure, ThemeColors};
use super::theme;
use super::{Context, HarnessError, Result};

/// Where snapshots live, relative to the repo root.
pub const SNAPSHOTS_DIR: &str = "parity/snapshots";
/// The run's effects in a snapshot directory.
pub const EFFECTS_FILE: &str = "effects.txt";
/// The extension of a capture's screen text ([`screen::render`]).
pub const SCREEN_EXTENSION: &str = "screen";
/// The extension of a capture's picture.
pub const SVG_EXTENSION: &str = "svg";
/// What a masked cell shows, in the default colors.
pub const MASK_SYMBOL: &str = "░";
/// The report's default colors, which the SVGs draw a default foreground and background in.
pub const DEFAULT_FG: (u8, u8, u8) = (0xd4, 0xd4, 0xd4);
pub const DEFAULT_BG: (u8, u8, u8) = (0x1e, 0x1e, 0x1e);
/// The suffix of what a run saw when it differs from the snapshot, in the report.
pub const NEW_SUFFIX: &str = ".new";
/// The suffix of the committed snapshot's `.screen` copied next to a run that differs from it, for the report.
pub const SNAPSHOT_SUFFIX: &str = ".snapshot";

/// `parity/snapshots/<scenario>/<size>/`.
pub fn snapshot_dir(repo: &Path, scenario: &str, size: Size) -> PathBuf {
    repo.join(SNAPSHOTS_DIR).join(scenario).join(size.to_string())
}

// ---------------------------------------------------------------------------------------------------------------
// Masking

/// What a capture's snapshot leaves out.
pub struct Masks<'a> {
    /// The capture's `ignore.boxes`: the interior of every box whose title contains one is masked.
    pub boxes: &'a [String],
    /// The capture's `mask`: every match on a row is masked.
    pub patterns: &'a [Regex],
    /// The run directory, as the harness made it and in canonical form: masked wherever it appears on a row.
    pub run_dirs: &'a [String],
}

/// `grid` with the cells `masks` cover replaced by [`MASK_SYMBOL`] in the default style, and a hidden cursor moved to
/// `0 0` (its position isn't stable, and no check reads it). `structure` is `grid`'s, for the boxes. A masked wide
/// character's continuation cells are masked too, so the row keeps its width.
pub fn mask(grid: &Grid, structure: &Structure, masks: &Masks<'_>) -> Grid {
    let interiors: Vec<[usize; 4]> = structure
        .boxes
        .iter()
        .filter(|p| masks.boxes.iter().any(|b| p.title.contains(b.as_str())))
        .map(|p| p.rect.map(usize::from))
        .collect();
    let literal: Vec<Regex> = masks
        .run_dirs
        .iter()
        .filter(|d| !d.is_empty())
        // An escaped literal always compiles.
        .filter_map(|d| Regex::new(&regex::escape(d)).ok())
        .collect();
    let mut out = grid.clone();
    for (row, line) in out.lines.iter_mut().enumerate() {
        let mut masked: Vec<bool> = (0..line.cells.len())
            .map(|col| {
                interiors
                    .iter()
                    .any(|&[c, r, w, h]| row > r && row + 1 < r + h && col > c && col + 1 < c + w)
            })
            .collect();
        let text: String = line.cells.concat();
        let mut starts = Vec::with_capacity(line.cells.len());
        let mut at = 0;
        for cell in &line.cells {
            starts.push(at);
            at += cell.len();
        }
        for re in masks.patterns.iter().chain(&literal) {
            for m in re.find_iter(&text) {
                for (col, cell) in line.cells.iter().enumerate() {
                    let start = starts[col];
                    if !cell.is_empty() && start < m.end() && start + cell.len() > m.start() {
                        masked[col] = true;
                    }
                }
            }
        }
        // A wide character's continuation cells follow it.
        for col in 1..masked.len() {
            if line.cells[col].is_empty() && masked[col - 1] {
                masked[col] = true;
            }
        }
        if !masked.contains(&true) {
            continue;
        }
        let mut styles = cell_styles(line);
        for (col, hide) in masked.iter().enumerate() {
            if *hide {
                MASK_SYMBOL.clone_into(&mut line.cells[col]);
                styles[col] = Style::default();
            }
        }
        line.runs = runs(&styles);
    }
    if !out.cursor.visible {
        out.cursor = Cursor {
            row: 0,
            col: 0,
            visible: false,
        };
    }
    out
}

/// Each cell's style in a row.
fn cell_styles(line: &Line) -> Vec<Style> {
    line.runs
        .iter()
        .flat_map(|run| std::iter::repeat_n(run.style.clone(), usize::from(run.len)))
        .collect()
}

/// Consecutive equal styles as runs, as a grid dump has them.
fn runs(styles: &[Style]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (col, style) in (0u16..).zip(styles) {
        match runs.last_mut() {
            Some(run) if run.style == *style => run.len += 1,
            _ => runs.push(Run {
                col,
                len: 1,
                style: style.clone(),
            }),
        }
    }
    runs
}

/// The capture's masks with the run's directories.
fn masks_of<'a>(capture: &'a Capture, run_dirs: &'a [String]) -> Masks<'a> {
    Masks {
        boxes: &capture.ignore.boxes,
        patterns: &capture.mask,
        run_dirs,
    }
}

/// The run directory `dir` as the harness made it and, when it differs, its canonical form (macOS's
/// `/private/var/...` for `/var/...`).
pub fn run_dir_forms(dir: &Path) -> Vec<String> {
    let mut forms = vec![dir.to_string_lossy().into_owned()];
    if let Ok(canonical) = std::fs::canonicalize(dir) {
        let canonical = canonical.to_string_lossy().into_owned();
        if !forms.contains(&canonical) {
            forms.push(canonical);
        }
    }
    forms
}

// ---------------------------------------------------------------------------------------------------------------
// Effects

/// `effects.txt`: one line per distinct thing the run did outside the screen, sorted, duplicates dropped. Each
/// served request as `<METHOD> <path>[?<query>][ accept=<accept>][ <body>]` (query keys sorted, the accept left out
/// when it's `json`, the body compact JSON with sorted object keys), each fake-ssh line as `ssh <service> <path>`,
/// and each URL opened in the browser as `open <url>`.
pub fn effects(fake_gh_log: &str, browser_log: &str) -> String {
    let mut lines = BTreeSet::new();
    for entry in RequestLog::parse(fake_gh_log).entries() {
        if entry.is_ssh() {
            lines.insert(format!(
                "ssh {} {}",
                entry.service.as_deref().unwrap_or("?"),
                entry.path.as_deref().unwrap_or("?")
            ));
            continue;
        }
        let (true, Some(method), Some(path)) = (entry.is_served(), &entry.method, &entry.path) else {
            continue;
        };
        let mut line = format!(
            "{method} {path}{}",
            query_suffix(&entry.query.clone().unwrap_or_default())
        );
        let accept = entry.accept.as_deref().map(Accept::normalize).unwrap_or_default();
        if accept != Accept::Json {
            line.push_str(" accept=");
            line.push_str(accept.as_str());
        }
        if let Some(body) = entry.body.as_ref().filter(|b| !b.is_null()) {
            line.push(' ');
            line.push_str(&sorted_json(body));
        }
        lines.insert(line);
    }
    for line in fake_gh::browser::parse(browser_log) {
        if let BrowserLine::Opened { url } = line {
            lines.insert(format!("open {url}"));
        }
    }
    lines.into_iter().fold(String::new(), |mut out, line| {
        out.push_str(&line);
        out.push('\n');
        out
    })
}

/// `?k=v&k=w` for a logged query, in the log's (sorted) key order, or nothing when it's empty.
fn query_suffix(query: &Query) -> String {
    let pairs: Vec<String> = query
        .iter()
        .flat_map(|(k, v)| match v {
            QueryValue::One(v) => vec![format!("{k}={v}")],
            QueryValue::Many(vs) => vs.iter().map(|v| format!("{k}={v}")).collect(),
        })
        .collect();
    if pairs.is_empty() {
        String::new()
    } else {
        format!("?{}", pairs.join("&"))
    }
}

/// `value` as compact JSON with every object's keys sorted.
fn sorted_json(value: &Value) -> String {
    fn sort(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                Value::Object(keys.into_iter().map(|k| (k.clone(), sort(&map[k]))).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    sort(value).to_string()
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering

/// `grid` as a [`Screen`], for [`Screen::to_svg`]: `#rrggbb` colors as RGB, `idx:<n>` as indexed, `null` as the
/// default, and `inverse` as `reversed`.
pub fn to_screen(grid: &Grid) -> Screen {
    let mut out = Screen::new(grid.cols, grid.rows);
    for (y, line) in (0u16..).zip(&grid.lines) {
        let styles = cell_styles(line);
        for (x, text) in (0u16..).zip(&line.cells) {
            let Some(cell) = out.cell_mut(x, y) else { continue };
            cell.symbol.clone_from(text);
            if let Some(style) = styles.get(usize::from(x)) {
                cell.style = CellStyle {
                    fg: color(style.fg.as_deref()),
                    bg: color(style.bg.as_deref()),
                    bold: style.bold,
                    italic: style.italic,
                    underline: style.underline,
                    reversed: style.inverse,
                    ..CellStyle::default()
                };
            }
        }
    }
    out
}

fn color(text: Option<&str>) -> Color {
    let Some(text) = text else {
        return Color::Default;
    };
    if let Some(n) = text.strip_prefix("idx:").and_then(|n| n.parse().ok()) {
        return Color::Indexed(n);
    }
    let hex = |range: std::ops::Range<usize>| text.get(range).and_then(|h| u8::from_str_radix(h, 16).ok());
    match (text.len(), text.starts_with('#'), hex(1..3), hex(3..5), hex(5..7)) {
        (7, true, Some(r), Some(g), Some(b)) => Color::Rgb(r, g, b),
        _ => Color::Default,
    }
}

/// The SVG of a capture's snapshot, titled `<scenario> · <capture> · <cols>x<rows>`.
pub fn svg(scenario: &str, capture: &str, grid: &Grid) -> String {
    to_screen(grid).to_svg(&SvgOptions {
        title: format!("{scenario} · {capture} · {}x{}", grid.cols, grid.rows),
        default_fg: DEFAULT_FG,
        default_bg: DEFAULT_BG,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Comparing

/// How a run's capture compares with its snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Unchanged,
    /// `summary` says what differs (`rows 12-14 differ`); `diff` is a unified diff of the two `.screen` texts;
    /// `stored` is the snapshot's.
    Differs {
        summary: String,
        diff: String,
        stored: String,
    },
    Missing,
}

/// Compares `masked` (a run's masked capture) with `<dir>/<capture>.screen`.
pub fn compare(dir: &Path, capture: &str, masked: &Grid) -> Result<Outcome> {
    let path = dir.join(format!("{capture}.{SCREEN_EXTENSION}"));
    let stored = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Outcome::Missing),
        Err(e) => return Err(HarnessError(format!("reading {}: {e}", path.display()))),
    };
    let text = screen::render(masked).with_context(|| format!("rendering {capture}"))?;
    if text == stored {
        return Ok(Outcome::Unchanged);
    }
    let summary = match screen::parse(&stored) {
        Ok(old) => grid_difference(&old, masked),
        Err(e) => format!("the snapshot doesn't parse ({e})"),
    };
    Ok(Outcome::Differs {
        summary,
        diff: diff::labeled_diff(&stored, &text, "snapshot", "run"),
        stored,
    })
}

/// What differs between two grids, in a few words: the size, the rows (0-based, first to last), or the cursor.
pub fn grid_difference(a: &Grid, b: &Grid) -> String {
    if (a.cols, a.rows) != (b.cols, b.rows) {
        return format!("the size differs ({}x{}, then {}x{})", a.cols, a.rows, b.cols, b.rows);
    }
    let differing: Vec<usize> = a
        .lines
        .iter()
        .zip(&b.lines)
        .enumerate()
        .filter(|(_, (la, lb))| la != lb)
        .map(|(i, _)| i)
        .collect();
    match (differing.first(), differing.last()) {
        (Some(first), Some(last)) => format!("rows {first}-{last} differ"),
        _ if a.cursor != b.cursor => "the cursor differs".to_owned(),
        _ => "the screens differ".to_owned(),
    }
}

/// What differs between two `effects.txt` texts: the lines only in each, or `None` when they're equal.
pub fn effects_difference(snapshot: &str, run: &str) -> Option<String> {
    let set = |text: &str| -> BTreeSet<String> { text.lines().map(str::to_owned).collect() };
    let (old, new) = (set(snapshot), set(run));
    let describe = |lines: Vec<&String>, where_: &str| {
        let n = lines.len();
        let joined: Vec<&str> = lines.iter().map(|l| l.as_str()).collect();
        format!(
            "{n} line{} only in {where_}: {}",
            if n == 1 { "" } else { "s" },
            joined.join(", ")
        )
    };
    let mut parts = Vec::new();
    let only_run: Vec<&String> = new.difference(&old).collect();
    if !only_run.is_empty() {
        parts.push(describe(only_run, "the run"));
    }
    let only_snapshot: Vec<&String> = old.difference(&new).collect();
    if !only_snapshot.is_empty() {
        parts.push(describe(only_snapshot, "the snapshot"));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

// ---------------------------------------------------------------------------------------------------------------
// One run

/// What one run captured, as its snapshot holds it.
#[derive(Debug, Clone)]
pub struct Captured {
    /// Each capture's masked grid, in step order. A capture the run didn't dump is left out.
    pub screens: Vec<(String, Grid)>,
    /// `effects.txt`.
    pub effects: String,
}

impl Captured {
    fn screen(&self, name: &str) -> Option<&Grid> {
        self.screens.iter().find(|(n, _)| n == name).map(|(_, g)| g)
    }

    /// The files of its snapshot directory, by name, in order: each capture's `.screen` and `.svg`, then
    /// `effects.txt`.
    fn files(&self, scenario: &str) -> Result<Vec<(String, String)>> {
        let mut files = Vec::new();
        for (name, grid) in &self.screens {
            let text = screen::render(grid).with_context(|| format!("rendering {name}"))?;
            files.push((format!("{name}.{SCREEN_EXTENSION}"), text));
            files.push((format!("{name}.{SVG_EXTENSION}"), svg(scenario, name, grid)));
        }
        files.push((EFFECTS_FILE.to_owned(), self.effects.clone()));
        Ok(files)
    }
}

/// Reads back what a finished run left in `out`: each capture's grid dump, masked with the capture's masks and
/// `run_dirs`, and its effects. `theme` finds the boxes.
pub fn capture_run(scenario: &Scenario, out: &Path, theme: &ThemeColors, run_dirs: &[String]) -> Result<Captured> {
    let mut screens = Vec::new();
    for step in &scenario.steps {
        let Step::Capture(capture) = step else { continue };
        let path = out.join(format!("{}.grid.json", capture.name));
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(HarnessError(format!("reading {}: {e}", path.display()))),
        };
        let grid: Grid = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let structure = structure::extract(&grid, theme);
        screens.push((
            capture.name.clone(),
            mask(&grid, &structure, &masks_of(capture, run_dirs)),
        ));
    }
    let read = |name: &str| -> Result<String> {
        let path = out.join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(HarnessError(format!("reading {}: {e}", path.display()))),
        }
    };
    Ok(Captured {
        screens,
        effects: effects(&read("fake-gh.log")?, &read(env::BROWSER_LOG)?),
    })
}

/// The first `[expect]` failure of the run in `out`, as `<capture>: <mismatch>`, or `None`.
pub fn expect_failure(scenario: &Scenario, out: &Path, theme: &ThemeColors) -> Result<Option<String>> {
    let browser = match std::fs::read_to_string(out.join(env::BROWSER_LOG)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(HarnessError(format!("reading {}: {e}", env::BROWSER_LOG))),
    };
    for name in scenario.capture_names() {
        let Some(expect) = scenario.expect.get(name) else {
            continue;
        };
        let path = out.join(format!("{name}.grid.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let grid: Grid = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let mut found = check::check_expect(&structure::extract(&grid, theme), expect);
        found.extend(check::check_opened(
            &fake_gh::browser::opened_by(&browser, name),
            expect,
        ));
        if let Some(m) = found.first() {
            return Ok(Some(format!("{name}: {}", m.describe())));
        }
    }
    Ok(None)
}

/// One snapshot mismatch, on the capture it goes on.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotMismatch {
    pub capture: String,
    pub mismatch: Mismatch,
}

/// Compares a run with its snapshot at `repo`: each capture's masked screen with its `.screen`, and the run's effects
/// with `effects.txt` (a mismatch on the scenario's last capture). What differs is written to the run's report dir
/// `out`: `<capture>.screen.new`, `<capture>.svg.new`, `<capture>.snapshot.diff.txt` and, when there is one, the
/// snapshot's own `<capture>.screen.snapshot`, which the report shows beside the run; and `effects.txt.new` and
/// `effects.diff.txt`.
pub fn check_run(
    repo: &Path,
    scenario: &Scenario,
    size: Size,
    captured: &Captured,
    out: &Path,
) -> Result<Vec<SnapshotMismatch>> {
    let dir = snapshot_dir(repo, &scenario.name, size);
    let write = |name: &str, text: &str| {
        let path = out.join(name);
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    };
    let mut found = Vec::new();
    let mut last = None;
    for name in scenario.capture_names() {
        last = Some(name);
        let Some(grid) = captured.screen(name) else { continue };
        let (message, diff) = match compare(&dir, name, grid)? {
            Outcome::Unchanged => continue,
            Outcome::Differs { summary, diff, stored } => {
                write(&format!("{name}.{SCREEN_EXTENSION}{SNAPSHOT_SUFFIX}"), &stored)?;
                (
                    format!(
                        "{summary}; run cargo xtask parity --accept {} and review the diff",
                        scenario.name
                    ),
                    diff,
                )
            }
            Outcome::Missing => {
                let text = screen::render(grid).with_context(|| format!("rendering {name}"))?;
                (
                    format!("missing; run cargo xtask parity --accept {}", scenario.name),
                    diff::labeled_diff("", &text, "snapshot", "run"),
                )
            }
        };
        let text = screen::render(grid).with_context(|| format!("rendering {name}"))?;
        write(&format!("{name}.{SCREEN_EXTENSION}{NEW_SUFFIX}"), &text)?;
        write(
            &format!("{name}.{SVG_EXTENSION}{NEW_SUFFIX}"),
            &svg(&scenario.name, name, grid),
        )?;
        write(&format!("{name}.snapshot.diff.txt"), &diff)?;
        found.push(SnapshotMismatch {
            capture: name.to_owned(),
            mismatch: check::Mismatch::snapshot(check::Kind::Snapshot, message),
        });
    }
    let Some(last) = last else {
        return Ok(found);
    };
    let path = dir.join(EFFECTS_FILE);
    let message = match std::fs::read_to_string(&path) {
        Ok(stored) => effects_difference(&stored, &captured.effects).map(|d| {
            write(
                "effects.diff.txt",
                &diff::labeled_diff(&stored, &captured.effects, "snapshot", "run"),
            )
            .map(|()| d)
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(Ok(format!(
            "missing; run cargo xtask parity --accept {}",
            scenario.name
        ))),
        Err(e) => return Err(HarnessError(format!("reading {}: {e}", path.display()))),
    };
    if let Some(message) = message.transpose()? {
        write(&format!("{EFFECTS_FILE}{NEW_SUFFIX}"), &captured.effects)?;
        found.push(SnapshotMismatch {
            capture: last.to_owned(),
            mismatch: check::Mismatch::snapshot(check::Kind::Effects, message),
        });
    }
    Ok(found)
}

// ---------------------------------------------------------------------------------------------------------------
// Problems: what `xtask/tests/snapshots.rs` checks

/// Something wrong with the committed snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A file a scenario's capture or size needs isn't there (`startup/160x48/main.screen`).
    Missing { scenario: String, file: String },
    /// Something under `parity/snapshots/` no scenario, size or capture accounts for (repo-relative, `/` at the end
    /// of a directory).
    Orphaned { path: String, reason: String },
    /// A `.screen` that doesn't parse (`startup/160x48/main.screen:3: ...`).
    Invalid { file: String, message: String },
    /// A `.svg` that isn't the rendering of the `.screen` next to it.
    StaleSvg { scenario: String, file: String },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Missing { scenario, file } => {
                write!(
                    f,
                    "missing snapshot: {file}; run cargo xtask parity --accept {scenario}"
                )
            }
            Problem::Orphaned { path, reason } => {
                write!(
                    f,
                    "orphaned snapshot: {path} ({reason}); cargo xtask parity --accept deletes it"
                )
            }
            Problem::Invalid { file, message } => write!(f, "invalid snapshot: {file}:{message}"),
            Problem::StaleSvg { scenario, file } => write!(
                f,
                "stale snapshot: {file} isn't the rendering of its .screen; run cargo xtask parity --accept {scenario}"
            ),
        }
    }
}

/// The problems with the snapshots of `scenarios` under `repo`: missing and orphaned files, `.screen`s that don't
/// parse and `.svg`s that aren't their `.screen`'s rendering. With `all`, directories of scenarios that don't exist
/// are orphans too. Reads files only; nothing runs.
pub fn check(repo: &Path, scenarios: &[Scenario], all: bool) -> Result<Vec<Problem>> {
    let mut problems: Vec<Problem> = orphans(repo, scenarios, all)?
        .into_iter()
        .map(|(path, reason)| Problem::Orphaned { path, reason })
        .collect();
    for scenario in scenarios {
        for &size in &scenario.sizes {
            let dir = snapshot_dir(repo, &scenario.name, size);
            let shown = |file: &str| format!("{}/{size}/{file}", scenario.name);
            let missing = |file: &str| Problem::Missing {
                scenario: scenario.name.clone(),
                file: shown(file),
            };
            for name in scenario.capture_names() {
                let screen_file = format!("{name}.{SCREEN_EXTENSION}");
                let svg_file = format!("{name}.{SVG_EXTENSION}");
                let Some(text) = read_optional(&dir.join(&screen_file))? else {
                    problems.push(missing(&screen_file));
                    continue;
                };
                let grid = match screen::parse(&text) {
                    Ok(grid) => grid,
                    Err(e) => {
                        problems.push(Problem::Invalid {
                            file: shown(&screen_file),
                            message: format!("{}: {}", e.line, e.message),
                        });
                        continue;
                    }
                };
                match read_optional(&dir.join(&svg_file))? {
                    None => problems.push(missing(&svg_file)),
                    Some(svg_text) if svg_text != svg(&scenario.name, name, &grid) => {
                        problems.push(Problem::StaleSvg {
                            scenario: scenario.name.clone(),
                            file: shown(&svg_file),
                        });
                    }
                    Some(_) => {}
                }
            }
            if !dir.join(EFFECTS_FILE).is_file() {
                problems.push(missing(EFFECTS_FILE));
            }
        }
    }
    Ok(problems)
}

/// Entries under `parity/snapshots/` that the selected scenarios don't account for, as repo-relative paths (a
/// directory's ending in `/`) with the reason. With `all`, directories of scenarios that aren't selected count too.
pub fn orphans(repo: &Path, scenarios: &[Scenario], all: bool) -> Result<Vec<(String, String)>> {
    let root = repo.join(SNAPSHOTS_DIR);
    let mut out = Vec::new();
    if all {
        for path in list_dir(&root)? {
            let name = file_name(&path);
            let reason = match scenarios.iter().find(|s| s.name == name) {
                _ if !path.is_dir() => "not part of a snapshot".to_owned(),
                None => "no such scenario".to_owned(),
                Some(_) => continue,
            };
            out.push((display(repo, &path), reason));
        }
    }
    for scenario in scenarios {
        let dir = root.join(&scenario.name);
        let sizes: BTreeSet<String> = scenario.sizes.iter().map(Size::to_string).collect();
        let mut files: BTreeSet<String> = BTreeSet::from([EFFECTS_FILE.to_owned()]);
        for name in scenario.capture_names() {
            files.insert(format!("{name}.{SCREEN_EXTENSION}"));
            files.insert(format!("{name}.{SVG_EXTENSION}"));
        }
        for path in list_dir(&dir)? {
            let name = file_name(&path);
            if !(path.is_dir() && sizes.contains(&name)) {
                let reason = if path.is_dir() {
                    format!("{} has no size {name}", scenario.name)
                } else {
                    "not part of a snapshot".to_owned()
                };
                out.push((display(repo, &path), reason));
                continue;
            }
            for file in list_dir(&path)? {
                let name = file_name(&file);
                if file.is_file() && files.contains(&name) {
                    continue;
                }
                let capture = name
                    .strip_suffix(&format!(".{SCREEN_EXTENSION}"))
                    .or_else(|| name.strip_suffix(&format!(".{SVG_EXTENSION}")));
                let reason = match capture {
                    Some(c) if file.is_file() => format!("{} has no capture {c}", scenario.name),
                    _ => "not part of a snapshot".to_owned(),
                };
                out.push((display(repo, &file), reason));
            }
        }
    }
    Ok(out)
}

/// The entries of `dir`, sorted, without dotfiles. Empty when `dir` doesn't exist.
fn list_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(HarnessError(format!("listing {}: {e}", dir.display()))),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry.with_context(|| format!("listing {}", dir.display()))?.path();
        if !file_name(&path).starts_with('.') {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `path` relative to `repo`, `/`-separated, a directory's ending in `/`.
fn display(repo: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(repo).unwrap_or(path);
    let text = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    if path.is_dir() { format!("{text}/") } else { text }
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(HarnessError(format!("reading {}: {e}", path.display()))),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Accepting

/// How to accept.
pub struct AcceptOptions {
    /// Every scenario is selected, so snapshot directories of scenarios that don't exist are orphans.
    pub all: bool,
    /// How many units run at once.
    pub jobs: usize,
    /// Copy each run's `home/` and `work/` into its dump directory.
    pub keep_home: bool,
}

/// What happened to one snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptOutcome {
    /// Two runs agreed and the snapshot was written.
    Accepted,
    /// Two runs agreed with each other and with the snapshot already there.
    Unchanged,
    /// Nothing was written.
    NotAccepted {
        /// The first disagreement, e.g. `run 2 differs from run 1: main: rows 3-3 differ`.
        reason: String,
        /// A unified diff of what the two runs disagree on, when there is one.
        diff: String,
    },
}

impl fmt::Display for AcceptOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcceptOutcome::Accepted => f.write_str("accepted (2 runs)"),
            AcceptOutcome::Unchanged => f.write_str("unchanged"),
            AcceptOutcome::NotAccepted { reason, .. } => write!(f, "not accepted: {reason}"),
        }
    }
}

/// One snapshot's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptUnit {
    pub scenario: String,
    pub size: Size,
    pub outcome: AcceptOutcome,
}

/// What [`accept`] did.
#[derive(Debug, Clone)]
pub struct AcceptReport {
    /// One per selected scenario, per size, in order.
    pub outcomes: Vec<AcceptUnit>,
    /// The orphans deleted, repo-relative.
    pub deleted: Vec<String>,
}

impl AcceptReport {
    /// Whether every selected snapshot was written or was already right.
    pub fn all_accepted(&self) -> bool {
        self.outcomes
            .iter()
            .all(|u| !matches!(u.outcome, AcceptOutcome::NotAccepted { .. }))
    }

    /// `N accepted, N unchanged, N not accepted, N orphan(s) deleted`.
    pub fn counts(&self) -> String {
        let count = |pred: fn(&AcceptOutcome) -> bool| self.outcomes.iter().filter(|u| pred(&u.outcome)).count();
        format!(
            "{} accepted, {} unchanged, {} not accepted, {} orphan(s) deleted",
            count(|o| matches!(o, AcceptOutcome::Accepted)),
            count(|o| matches!(o, AcceptOutcome::Unchanged)),
            count(|o| matches!(o, AcceptOutcome::NotAccepted { .. })),
            self.deleted.len(),
        )
    }
}

/// Accepts the snapshots of `scenarios` under `repo`: for each scenario, at each of its sizes, two runs (dumps in
/// `<out>/<scenario>/<size>/run-N/`) that must pass and agree, then the snapshot is written (or left alone when it's
/// unchanged). Then deletes orphans. Prints one line per snapshot as it finishes. A harness
/// error in one unit doesn't stop the others; the first is returned after all of them ran.
pub fn accept(
    repo: &Path,
    scenarios: &[Scenario],
    paths: &Paths,
    out: &Path,
    options: &AcceptOptions,
) -> Result<AcceptReport> {
    let units: Vec<(&Scenario, Size)> = scenarios
        .iter()
        .flat_map(|s| s.sizes.iter().map(move |&size| (s, size)))
        .collect();
    let results = cli::parallel_map(&units, options.jobs, |&(scenario, size)| {
        let result = accept_unit(repo, scenario, size, paths, out, options);
        let mut text = format!("{} {size}: ", scenario.name);
        match &result {
            Ok(outcome) => {
                text.push_str(&outcome.to_string());
                text.push('\n');
                if let AcceptOutcome::NotAccepted { diff, .. } = outcome {
                    text.push_str(diff);
                }
            }
            Err(e) => {
                text.push_str("harness error: ");
                text.push_str(&e.to_string());
                text.push('\n');
            }
        }
        // One write per unit, so lines from parallel units don't interleave.
        print!("{text}");
        result
    });

    let mut outcomes = Vec::new();
    let mut first_error = None;
    for (&(scenario, size), result) in units.iter().zip(results) {
        match result {
            Some(Ok(outcome)) => outcomes.push(AcceptUnit {
                scenario: scenario.name.clone(),
                size,
                outcome,
            }),
            Some(Err(e)) => {
                first_error.get_or_insert(e);
            }
            None => {
                first_error.get_or_insert_with(|| {
                    HarnessError(format!("{} {size}: the worker thread panicked", scenario.name))
                });
            }
        }
    }

    let mut deleted = Vec::new();
    for (path, _) in orphans(repo, scenarios, options.all)? {
        remove(&repo_path(repo, &path))?;
        println!("deleted orphaned snapshot: {path}");
        deleted.push(path);
    }
    if let Some(e) = first_error {
        return Err(e);
    }
    Ok(AcceptReport { outcomes, deleted })
}

/// One finished run, read back.
struct AcceptRun {
    label: String,
    captured: Captured,
}

fn accept_unit(
    repo: &Path,
    scenario: &Scenario,
    size: Size,
    paths: &Paths,
    out: &Path,
    options: &AcceptOptions,
) -> Result<AcceptOutcome> {
    let colors = theme::scenario_colors(scenario, &paths.roots)?;
    let theme = colors.theme;
    let dumps = out.join(&scenario.name).join(size.to_string());
    if dumps.exists() {
        std::fs::remove_dir_all(&dumps).with_context(|| format!("wiping {}", dumps.display()))?;
    }
    let run_options = RunOptions {
        sizes: &[],
        retries: 0,
        keep_home: options.keep_home,
    };
    // A run that can't be a snapshot on its own: it failed, or its `[expect]` did.
    let run = |n: u32| -> Result<std::result::Result<AcceptRun, String>> {
        let label = format!("run {n}");
        let run_out = dumps.join(format!("run-{n}"));
        let run = cli::run_app(scenario, size, paths, &run_out, &run_options, &colors.focus)?;
        if !run.status.passed() {
            return Ok(Err(format!("{label} {}", run.status.describe())));
        }
        if let Some(failure) = expect_failure(scenario, &run_out, &theme)? {
            return Ok(Err(format!("{label} [expect] {failure}")));
        }
        Ok(Ok(AcceptRun {
            label,
            captured: capture_run(scenario, &run_out, &theme, &run.run_dirs)?,
        }))
    };
    let not_accepted = |reason: String, diff: String| Ok(AcceptOutcome::NotAccepted { reason, diff });

    let first = match run(1)? {
        Ok(first) => first,
        Err(reason) => return not_accepted(reason, String::new()),
    };
    let second = match run(2)? {
        Ok(second) => second,
        Err(reason) => return not_accepted(reason, String::new()),
    };
    if let Some((reason, diff)) = disagreement(&first, &second)? {
        return not_accepted(reason, diff);
    }
    write_snapshot(repo, scenario, size, &first.captured)
}

/// The first way two runs disagree, with a diff of it: their captures, a capture's masked screen, or their effects.
fn disagreement(a: &AcceptRun, b: &AcceptRun) -> Result<Option<(String, String)>> {
    let prefix = format!("{} differs from {}", b.label, a.label);
    let names = |run: &AcceptRun| -> Vec<String> { run.captured.screens.iter().map(|(n, _)| n.clone()).collect() };
    let (a_names, b_names) = (names(a), names(b));
    if a_names != b_names {
        return Ok(Some((
            format!("{prefix}: captures {b_names:?}, not {a_names:?}"),
            String::new(),
        )));
    }
    for (name, ga) in &a.captured.screens {
        let Some(gb) = b.captured.screen(name) else { continue };
        let (ta, tb) = (
            screen::render(ga).with_context(|| format!("rendering {name}"))?,
            screen::render(gb).with_context(|| format!("rendering {name}"))?,
        );
        if ta != tb {
            return Ok(Some((
                format!("{prefix}: {name}: {}", grid_difference(ga, gb)),
                diff::labeled_diff(&ta, &tb, &a.label, &b.label),
            )));
        }
    }
    if let Some(difference) = effects_difference(&a.captured.effects, &b.captured.effects) {
        return Ok(Some((
            format!("{prefix}: effects: {difference}"),
            diff::labeled_diff(&a.captured.effects, &b.captured.effects, &a.label, &b.label),
        )));
    }
    Ok(None)
}

/// Writes `captured` as the snapshot of `scenario` at `size`, replacing the directory in one rename, unless the
/// directory already holds exactly these files.
fn write_snapshot(repo: &Path, scenario: &Scenario, size: Size, captured: &Captured) -> Result<AcceptOutcome> {
    let dir = snapshot_dir(repo, &scenario.name, size);
    let files = captured.files(&scenario.name)?;
    let mut unchanged = list_dir(&dir)?.len() == files.len();
    for (name, text) in &files {
        if !unchanged {
            break;
        }
        unchanged = read_optional(&dir.join(name))?.as_deref() == Some(text.as_str());
    }
    if unchanged {
        return Ok(AcceptOutcome::Unchanged);
    }
    let tmp = dir.with_file_name(format!("{size}.tmp"));
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp).with_context(|| format!("wiping {}", tmp.display()))?;
    }
    std::fs::create_dir_all(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
    for (name, text) in &files {
        let path = tmp.join(name);
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    replace_dir(&tmp, &dir)?;
    Ok(AcceptOutcome::Accepted)
}

/// Moves `from` to `to`, replacing `to`. A directory can't be renamed over a non-empty one, so the old one is moved
/// aside first; an interrupted replace leaves `<size>.old/` or `<size>.tmp/`, which the next `--accept` deletes as
/// orphans.
fn replace_dir(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        let aside = to.with_extension("old");
        if aside.exists() {
            std::fs::remove_dir_all(&aside).with_context(|| format!("wiping {}", aside.display()))?;
        }
        std::fs::rename(to, &aside).with_context(|| format!("moving {} aside", to.display()))?;
        std::fs::rename(from, to).with_context(|| format!("moving {} to {}", from.display(), to.display()))?;
        std::fs::remove_dir_all(&aside).with_context(|| format!("removing {}", aside.display()))
    } else {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::rename(from, to).with_context(|| format!("moving {} to {}", from.display(), to.display()))
    }
}

/// A repo-relative, `/`-separated path (directories end in `/`) on disk.
fn repo_path(repo: &Path, path: &str) -> PathBuf {
    path.split('/')
        .filter(|p| !p.is_empty())
        .fold(repo.to_owned(), |acc, p| acc.join(p))
}

fn remove(path: &Path) -> Result<()> {
    let result = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    result.with_context(|| format!("deleting {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::test_grid;

    fn style(fg: Option<&str>, bg: Option<&str>) -> Style {
        Style {
            fg: fg.map(str::to_owned),
            bg: bg.map(str::to_owned),
            ..Style::default()
        }
    }

    /// A grid of `rows`, every cell in `style`, the cursor hidden at `5 1`.
    fn plain(rows: &[&str], cols: u16, style: &Style) -> Grid {
        let lines = rows
            .iter()
            .map(|row| {
                let mut cells: Vec<String> = row.chars().map(|c| c.to_string()).collect();
                cells.resize(usize::from(cols), " ".to_owned());
                Line {
                    cells,
                    runs: vec![Run {
                        col: 0,
                        len: cols,
                        style: style.clone(),
                    }],
                }
            })
            .collect();
        Grid {
            version: 1,
            cols,
            rows: u16::try_from(rows.len()).unwrap(),
            cursor: Cursor {
                row: 1,
                col: 5,
                visible: false,
            },
            lines,
        }
    }

    fn no_masks() -> Masks<'static> {
        Masks {
            boxes: &[],
            patterns: &[],
            run_dirs: &[],
        }
    }

    fn theme() -> ThemeColors {
        ThemeColors::default()
    }

    #[test]
    fn masks_the_interior_of_an_ignored_box() {
        let grid = test_grid::parse(
            "┌─ Details ──┐┌─ Repos ─┐\n\
             │ secret 1   ││ kept    │\n\
             │ secret 2   ││         │\n\
             └────────────┘└─────────┘\n",
        );
        let structure = structure::extract(&grid, &theme());
        let boxes = ["Details".to_owned()];
        let masked = mask(
            &grid,
            &structure,
            &Masks {
                boxes: &boxes,
                ..no_masks()
            },
        );
        assert_eq!(
            masked.to_text(),
            "┌─ Details ──┐┌─ Repos ─┐\n\
             │░░░░░░░░░░░░││ kept    │\n\
             │░░░░░░░░░░░░││         │\n\
             └────────────┘└─────────┘\n"
        );
        // Masked cells are in the default style; the rest keep theirs.
        assert_eq!(masked.lines[1].runs[1].style, Style::default());
        assert_eq!(masked.lines[0], grid.lines[0]);
    }

    #[test]
    fn masks_regex_matches_and_the_run_dir_keeping_the_width() {
        let green = style(Some("#4ebf71"), Some("#121212"));
        let grid = plain(
            &[
                "Saved /tmp/lgp-ab12/home/x.svg ok",
                "Read-only: /private/tmp/lgp-ab12/home",
                "version 1.2.3 here",
            ],
            40,
            &green,
        );
        let structure = structure::extract(&grid, &theme());
        let patterns = [Regex::new(r"\d+\.\d+\.\d+").unwrap()];
        let run_dirs = ["/tmp/lgp-ab12".to_owned(), "/private/tmp/lgp-ab12".to_owned()];
        let masked = mask(
            &grid,
            &structure,
            &Masks {
                boxes: &[],
                patterns: &patterns,
                run_dirs: &run_dirs,
            },
        );
        assert_eq!(
            masked.text_rows(),
            [
                "Saved ░░░░░░░░░░░░░/home/x.svg ok",
                "Read-only: ░░░░░░░░░░░░░░░░░░░░░/home",
                "version ░░░░░ here",
            ]
        );
        for line in &masked.lines {
            assert_eq!(line.cells.len(), 40);
            assert_eq!(line.runs.iter().map(|r| usize::from(r.len)).sum::<usize>(), 40);
        }
        assert_eq!(masked.lines[2].runs[1].col, 8);
        assert_eq!(masked.lines[2].runs[1].len, 5);
        assert_eq!(masked.lines[2].runs[1].style, Style::default());
        assert_eq!(masked.lines[2].runs[2].style, green);
    }

    #[test]
    fn a_masked_wide_character_masks_its_continuation() {
        let mut grid = plain(&["ab"], 6, &Style::default());
        grid.lines[0].cells = ["a", "漢", "", "b", " ", " "].map(str::to_owned).to_vec();
        let structure = structure::extract(&grid, &theme());
        let patterns = [Regex::new("漢").unwrap()];
        let masked = mask(
            &grid,
            &structure,
            &Masks {
                patterns: &patterns,
                ..no_masks()
            },
        );
        assert_eq!(masked.lines[0].cells, ["a", "░", "░", "b", " ", " "]);
    }

    #[test]
    fn a_hidden_cursor_is_normalized_and_a_visible_one_kept() {
        let grid = plain(&["x"], 4, &Style::default());
        let structure = structure::extract(&grid, &theme());
        let masked = mask(&grid, &structure, &no_masks());
        assert_eq!(
            masked.cursor,
            Cursor {
                row: 0,
                col: 0,
                visible: false
            }
        );
        let mut visible = grid.clone();
        visible.cursor.visible = true;
        assert_eq!(mask(&visible, &structure, &no_masks()).cursor, visible.cursor);
    }

    #[test]
    fn effects_are_sorted_distinct_lines() {
        let log = [
            r#"{"seq":2,"argv":[],"command":"api","method":"GET","path":"/user/repos","query":{"page":"1","direction":"asc"},"accept":"json","fixture":"base/x.json","status":200,"miss":false,"error":null}"#,
            r#"{"seq":1,"argv":[],"command":"api","method":"POST","path":"/repos/o/r/issues","query":null,"accept":"json","body":{"title":"x","body":"y","labels":[{"z":1,"a":2}]},"fixture":"base/y.json","status":201,"miss":false,"error":null}"#,
            r#"{"seq":3,"argv":[],"command":"api","method":"GET","path":"/user/repos","query":{"direction":"asc","page":"1"},"accept":"json","fixture":"base/x.json","status":200,"miss":false,"error":null}"#,
            r#"{"seq":4,"argv":[],"command":"api","method":"GET","path":"/repos/o/r/pulls/1","accept":"diff","fixture":"base/d.json","status":200,"miss":false,"error":null}"#,
            r#"{"seq":5,"argv":[],"command":"auth status","fixture":null,"miss":false,"error":null}"#,
            r#"{"seq":6,"argv":[],"command":"api","method":"GET","path":"/missing","fixture":null,"status":404,"miss":true,"error":null}"#,
            r#"{"seq":7,"argv":[],"command":"ssh","service":"git-receive-pack","path":"o/r.git","status":0,"miss":false,"error":null}"#,
        ]
        .join("\n");
        let browser =
            "{\"url\":\"https://github.com/o/r\"}\n{\"capture\":\"main\"}\n{\"url\":\"https://github.com/o/r\"}\n";
        assert_eq!(
            effects(&log, browser),
            "GET /repos/o/r/pulls/1 accept=diff\n\
             GET /user/repos?direction=asc&page=1\n\
             POST /repos/o/r/issues {\"body\":\"y\",\"labels\":[{\"a\":2,\"z\":1}],\"title\":\"x\"}\n\
             open https://github.com/o/r\n\
             ssh git-receive-pack o/r.git\n"
        );
        assert_eq!(effects("", ""), "");
    }

    #[test]
    fn effects_differences_name_the_lines_on_each_side() {
        assert_eq!(effects_difference("a\nb\n", "a\nb\n"), None);
        assert_eq!(
            effects_difference("a\n", "a\nPOST /x {}\n").as_deref(),
            Some("1 line only in the run: POST /x {}")
        );
        assert_eq!(
            effects_difference("a\nb\nc\n", "d\n").as_deref(),
            Some("1 line only in the run: d; 3 lines only in the snapshot: a, b, c")
        );
    }

    #[test]
    fn compares_with_the_committed_screen() {
        let dir = tempfile::tempdir().unwrap();
        let grid = plain(&["one", "two", "three", "four"], 8, &Style::default());
        assert_eq!(compare(dir.path(), "main", &grid).unwrap(), Outcome::Missing);

        std::fs::write(dir.path().join("main.screen"), screen::render(&grid).unwrap()).unwrap();
        assert_eq!(compare(dir.path(), "main", &grid).unwrap(), Outcome::Unchanged);

        let mut changed = grid.clone();
        changed.lines[1].cells[0] = "T".into();
        changed.lines[2].cells[0] = "T".into();
        let Outcome::Differs { summary, diff, .. } = compare(dir.path(), "main", &changed).unwrap() else {
            panic!("not a difference");
        };
        assert_eq!(summary, "rows 1-2 differ");
        assert!(diff.starts_with("--- snapshot\n+++ run\n"), "{diff}");
        assert!(diff.contains("-two\n") && diff.contains("+Two\n"), "{diff}");
    }

    #[test]
    fn converts_grid_colors_and_attributes() {
        let mut grid = plain(&["ab", "c"], 4, &Style::default());
        grid.lines[0].runs = vec![
            Run {
                col: 0,
                len: 1,
                style: Style {
                    fg: Some("#4ebf71".into()),
                    bg: Some("idx:4".into()),
                    bold: true,
                    ..Style::default()
                },
            },
            Run {
                col: 1,
                len: 3,
                style: Style {
                    inverse: true,
                    italic: true,
                    underline: true,
                    ..Style::default()
                },
            },
        ];
        grid.lines[1].cells = ["漢", "", "c", " "].map(str::to_owned).to_vec();
        let screen = to_screen(&grid);
        let a = screen.cell(0, 0).unwrap();
        assert_eq!(a.symbol, "a");
        assert_eq!(a.style.fg, Color::Rgb(0x4e, 0xbf, 0x71));
        assert_eq!(a.style.bg, Color::Indexed(4));
        assert!(a.style.bold && !a.style.reversed);
        let b = screen.cell(1, 0).unwrap();
        assert_eq!((b.style.fg, b.style.bg), (Color::Default, Color::Default));
        assert!(b.style.reversed && b.style.italic && b.style.underline && !b.style.bold);
        assert_eq!(screen.cell(0, 1).unwrap().symbol, "漢");
        assert_eq!(screen.cell(1, 1).unwrap().symbol, "");
        assert_eq!(screen.to_text(), "ab\n漢c");
    }

    #[test]
    fn a_masked_screen_as_svg() {
        let grid = test_grid::parse(
            "┌─ Log ────┐\n\
             │ 12:00 hi │\n\
             └──────────┘\n\
             --- styles\n\
             gggggggggggg\n\
             g..........g\n\
             gggggggggggg\n\
             --- legend\n\
             g fg=#4ebf71 bg=#121212\n",
        );
        let structure = structure::extract(&grid, &theme());
        let boxes = ["Log".to_owned()];
        let masked = mask(
            &grid,
            &structure,
            &Masks {
                boxes: &boxes,
                ..no_masks()
            },
        );
        insta::assert_snapshot!(svg("demo", "main", &masked));
    }
}
