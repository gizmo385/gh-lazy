//! The structural check: each capture's [`Structure`] against the scenario's `[expect.<capture>]` (per app), and
//! Python against Rust (cross-app). Results go to `<capture>.check.json` and feed `summary.json`.
//!
//! [`check_expect`] and [`compare`] are pure; [`check_size`] reads the grid dumps, extracts, checks and writes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use fake_gh::log::RequestLog;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ci::Finding;
use super::env::build_config;
use super::focus::FocusColors;
use super::grid::{GRID_VERSION, Grid};
use super::reference;
use super::runner::AppRunStatus;
use super::scenario::{App, Check, Expect, ExpectTabs, Ignore, Roots, Scenario, Step};
use super::screen;
use super::structure::{self, Binding, DEFAULT_THEME, Pane, Structure, Table, ThemeColors, Themes};
use super::summary::{CaptureSummary, CheckOutcome};
use super::{Context, HarnessError, Result};

/// What a mismatch is about: a cross-app [`Check`] or an `[expect]` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Titles,
    Tabs,
    Headers,
    Rows,
    Cursor,
    Footer,
    Focus,
    Contains,
    NotContains,
    /// The served requests (drift only: a live Python run against its reference).
    Requests,
    /// The non-`GET` requests each app sent (cross-app, once per size, on its last capture).
    Writes,
    /// The URLs an app had opened in the browser by a capture (`[expect] opened`).
    Opened,
}

impl From<Check> for Kind {
    fn from(c: Check) -> Self {
        match c {
            Check::Titles => Kind::Titles,
            Check::Tabs => Kind::Tabs,
            Check::Headers => Kind::Headers,
            Check::Rows => Kind::Rows,
            Check::Cursor => Kind::Cursor,
            Check::Footer => Kind::Footer,
            Check::Focus => Kind::Focus,
        }
    }
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Titles => "titles",
            Kind::Tabs => "tabs",
            Kind::Headers => "headers",
            Kind::Rows => "rows",
            Kind::Cursor => "cursor",
            Kind::Footer => "footer",
            Kind::Focus => "focus",
            Kind::Contains => "contains",
            Kind::NotContains => "not_contains",
            Kind::Requests => "requests",
            Kind::Writes => "writes",
            Kind::Opened => "opened",
        }
    }
}

/// One failed check. For an `[expect]` failure, the failing app's field holds what it showed, the other is `null`,
/// and `expected` holds the expectation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mismatch {
    pub check: Kind,
    /// The title of the box the mismatch is in, when it's about one box.
    #[serde(rename = "box")]
    pub in_box: Option<String>,
    pub python: Value,
    pub rust: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    pub message: String,
}

impl Mismatch {
    fn cross(check: Kind, in_box: Option<&str>, python: Value, rust: Value, message: String) -> Self {
        Self {
            check,
            in_box: in_box.map(str::to_owned),
            python,
            rust,
            expected: None,
            message,
        }
    }

    fn expect(app: App, check: Kind, in_box: Option<&str>, actual: Value, expected: Value, message: String) -> Self {
        let (python, rust) = match app {
            App::Python => (actual, Value::Null),
            App::Rust => (Value::Null, actual),
        };
        Self {
            check,
            in_box: in_box.map(str::to_owned),
            python,
            rust,
            expected: Some(expected),
            message,
        }
    }

    /// The one-line console form.
    pub fn describe(&self) -> String {
        match &self.in_box {
            Some(b) => format!("{} [{b}]: {}", self.check.as_str(), self.message),
            None => format!("{}: {}", self.check.as_str(), self.message),
        }
    }
}

/// `<capture>.check.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckFile {
    pub capture: String,
    /// `[cols, rows]`.
    pub size: [u16; 2],
    pub expect: Vec<Mismatch>,
    pub cross: Vec<Mismatch>,
    /// Where a live Python run disagrees with its fresh reference. `python` is the live value, `rust` the
    /// reference's. Left out when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drift: Vec<Mismatch>,
}

/// One disagreement between a live Python run and its fresh reference, on one capture
/// ([`reference::drift`](super::reference::drift)). It means the reference is wrong, so it counts against Python.
#[derive(Debug, Clone, PartialEq)]
pub struct Drift {
    pub capture: String,
    /// `python` is the live value and `rust` the reference's; the message doesn't say so yet.
    pub mismatch: Mismatch,
}

/// The note after every drift line: what drift means and how to fix it.
pub fn drift_hint(scenario: &str) -> String {
    format!(
        "the reference is wrong or nondeterministic: run cargo xtask parity --record --force {scenario} and review \
         the diff"
    )
}

fn titles(s: &Structure) -> Vec<&str> {
    s.boxes.iter().map(|b| b.title.as_str()).collect()
}

fn footer_strings(s: &Structure) -> Vec<String> {
    s.footer.iter().map(Binding::display).collect()
}

/// Whether the footer shown matches the expected one. When the footer reaches the screen's edge, the screen may cut
/// it off: the bindings shown must then be a prefix of the expected ones, and the last one may be cut mid-label.
fn footer_matches(actual: &[String], clipped: bool, expected: &[String]) -> bool {
    if !clipped || actual.len() > expected.len() {
        return actual == expected;
    }
    match actual.split_last() {
        None => expected.is_empty(),
        Some((last, rest)) => rest == &expected[..rest.len()] && expected[rest.len()].starts_with(last.as_str()),
    }
}

/// Checks one app's structure against `[expect.<capture>]`.
pub fn check_expect(app: App, s: &Structure, e: &Expect) -> Vec<Mismatch> {
    let mut out = Vec::new();
    if let Some(expected) = &e.pane_titles {
        let actual = titles(s);
        if actual != *expected {
            out.push(Mismatch::expect(
                app,
                Kind::Titles,
                None,
                json!(actual),
                json!(expected),
                format!("pane titles are {actual:?}, expected {expected:?}"),
            ));
        }
    }
    if let Some(expected) = e.focused.as_ref().filter(|f| s.focused != **f) {
        out.push(Mismatch::expect(
            app,
            Kind::Focus,
            None,
            json!(s.focused),
            json!(expected),
            format!("focused boxes are {:?}, expected {expected:?}", s.focused),
        ));
    }
    if let Some(expected) = &e.footer {
        let actual = footer_strings(s);
        if !footer_matches(&actual, s.footer_clipped, expected) {
            out.push(Mismatch::expect(
                app,
                Kind::Footer,
                None,
                json!(actual),
                json!(expected),
                format!("footer is {actual:?}, expected {expected:?}"),
            ));
        }
    }
    for needle in e.contains.iter().flatten() {
        if !s.screen.iter().any(|row| row.contains(needle.as_str())) {
            out.push(Mismatch::expect(
                app,
                Kind::Contains,
                None,
                Value::Null,
                json!(needle),
                format!("no row contains {needle:?}"),
            ));
        }
    }
    for needle in e.not_contains.iter().flatten() {
        if let Some(row) = s.screen.iter().find(|row| row.contains(needle.as_str())) {
            out.push(Mismatch::expect(
                app,
                Kind::NotContains,
                None,
                json!(row.trim()),
                json!(needle),
                format!("a row contains {needle:?}: {:?}", row.trim()),
            ));
        }
    }
    out.extend(e.tabs.as_ref().and_then(|t| expect_tabs(app, s, t)));
    out
}

/// Checks what one app had opened in the browser by a capture (`opened`, from fake-browser's log) against
/// `[expect.<capture>] opened`.
pub fn check_opened(app: App, opened: &[String], e: &Expect) -> Option<Mismatch> {
    let expected = e.opened.as_ref()?;
    (opened != expected.as_slice()).then(|| {
        Mismatch::expect(
            app,
            Kind::Opened,
            None,
            json!(opened),
            json!(expected),
            format!("opened {opened:?}, expected {expected:?}"),
        )
    })
}

/// What `app` had opened in the browser by capture `name` in the size dir `dir`, from its `browser.jsonl`. A run
/// without the log opened nothing.
fn opened_by(dir: &Path, app: App, name: &str) -> Result<Vec<String>> {
    let path = dir.join(app.name()).join(super::env::BROWSER_LOG);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(fake_gh::browser::opened_by(&text, name)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(HarnessError(format!("reading {}: {e}", path.display()))),
    }
}

fn expect_tabs(app: App, s: &Structure, t: &ExpectTabs) -> Option<Mismatch> {
    let expected = json!({"names": t.names, "active": t.active});
    let Some(b) = s.boxes.iter().find(|b| b.title.contains(t.in_box.as_str())) else {
        return Some(Mismatch::expect(
            app,
            Kind::Tabs,
            Some(&t.in_box),
            Value::Null,
            expected,
            format!("no box title contains {:?}", t.in_box),
        ));
    };
    let names_ok = t
        .names
        .as_ref()
        .is_none_or(|n| b.tabs.as_ref().is_some_and(|bt| bt.names == *n));
    let active_ok = t
        .active
        .as_ref()
        .is_none_or(|a| b.tabs.as_ref().is_some_and(|bt| bt.active.as_ref() == Some(a)));
    if names_ok && active_ok {
        return None;
    }
    let message = match &b.tabs {
        None => "the box has no tabs".to_owned(),
        Some(bt) => format!(
            "tabs are {:?} with {:?} active, expected {:?} with {:?} active",
            bt.names, bt.active, t.names, t.active
        ),
    };
    Some(Mismatch::expect(
        app,
        Kind::Tabs,
        Some(&b.title),
        json!(b.tabs),
        expected,
        message,
    ))
}

/// Two cells are equal when their trimmed text is, or when one is cut off with `…` and the other starts with the
/// part before it: truncation width is layout, not content.
fn cells_equal(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    let cut = |x: &str, y: &str| x.strip_suffix('…').is_some_and(|stem| y.starts_with(stem));
    a == b || cut(a, b) || cut(b, a)
}

/// Rows are equal cell by cell. With `last_clipped`, the last cells may also be one a prefix of the other, since the
/// screen may have cut the last column off.
fn rows_equal(a: &[String], b: &[String], last_clipped: bool) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).enumerate().all(|(i, (x, y))| {
            cells_equal(x, y) || (last_clipped && i + 1 == a.len() && prefix_either(x.trim(), y.trim()))
        })
}

fn prefix_either(a: &str, b: &str) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Lists compared across apps when either side may have been cut off at the screen's edge: equal up to the shorter
/// list, with the last entry compared there allowed to be a prefix of the other. Without `clipped`, exact equality.
fn clipped_lists_equal(a: &[String], b: &[String], clipped: bool) -> bool {
    if !clipped {
        return a == b;
    }
    let n = a.len().min(b.len());
    if n == 0 {
        return a.len() == b.len();
    }
    a[..n - 1] == b[..n - 1] && prefix_either(&a[n - 1], &b[n - 1])
}

/// Pairs boxes with equal titles across apps, the nth box with a title in one app with the nth in the other.
fn pair_boxes<'a>(py: &'a Structure, rs: &'a Structure) -> Vec<(&'a Pane, &'a Pane)> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    let mut pairs = Vec::new();
    for p in &py.boxes {
        let n = seen.entry(p.title.as_str()).or_default();
        if let Some(r) = rs.boxes.iter().filter(|r| r.title == p.title).nth(*n) {
            pairs.push((p, r));
        }
        *n += 1;
    }
    pairs
}

/// One mismatch when the ordered title lists differ, naming the boxes found in only one app.
fn compare_titles(py: &Structure, rs: &Structure) -> Option<Mismatch> {
    let (p, r) = (titles(py), titles(rs));
    if p == r {
        return None;
    }
    let only = |a: &[&str], b: &[&str]| -> Vec<String> {
        a.iter().filter(|t| !b.contains(t)).map(|t| format!("{t:?}")).collect()
    };
    let (only_py, only_rs) = (only(&p, &r), only(&r, &p));
    let message = if only_py.is_empty() && only_rs.is_empty() {
        format!("the boxes are in a different order: python {p:?}, rust {r:?}")
    } else {
        let mut parts = Vec::new();
        if !only_py.is_empty() {
            parts.push(format!("only in python: {}", only_py.join(", ")));
        }
        if !only_rs.is_empty() {
            parts.push(format!("only in rust: {}", only_rs.join(", ")));
        }
        parts.join("; ")
    };
    Some(Mismatch::cross(Kind::Titles, None, json!(p), json!(r), message))
}

/// The row count of table `i`, then each row.
fn compare_rows(title: &str, i: usize, pt: &Table, rt: &Table, out: &mut Vec<Mismatch>) {
    let clipped = pt.headers_clipped || rt.headers_clipped;
    if pt.rows.len() != rt.rows.len() {
        out.push(Mismatch::cross(
            Kind::Rows,
            Some(title),
            json!(pt.rows.len()),
            json!(rt.rows.len()),
            format!(
                "table {i} has {} rows in python and {} in rust",
                pt.rows.len(),
                rt.rows.len()
            ),
        ));
        return;
    }
    for (j, (pr, rr)) in pt.rows.iter().zip(&rt.rows).enumerate() {
        if !rows_equal(pr, rr, clipped) {
            out.push(Mismatch::cross(
                Kind::Rows,
                Some(title),
                json!(pr),
                json!(rr),
                format!("table {i} row {j} differs"),
            ));
        }
    }
}

/// The `headers`, `rows` and `cursor` checks for one pair of boxes.
fn compare_tables(p: &Pane, r: &Pane, wants: &impl Fn(Check) -> bool, out: &mut Vec<Mismatch>) {
    if wants(Check::Headers) {
        let ph: Vec<_> = p.tables.iter().map(|t| &t.headers).collect();
        let rh: Vec<_> = r.tables.iter().map(|t| &t.headers).collect();
        let same = p.tables.len() == r.tables.len()
            && p.tables.iter().zip(&r.tables).all(|(pt, rt)| {
                clipped_lists_equal(&pt.headers, &rt.headers, pt.headers_clipped || rt.headers_clipped)
            });
        if !same {
            out.push(Mismatch::cross(
                Kind::Headers,
                Some(&p.title),
                json!(ph),
                json!(rh),
                "table headers differ".into(),
            ));
        }
    }
    // A table missing in one app has a row count and a cursor too, and they don't match.
    let table_check = [Check::Rows, Check::Cursor].into_iter().find(|&c| wants(c));
    if let Some(check) = table_check.filter(|_| p.tables.len() != r.tables.len()) {
        out.push(Mismatch::cross(
            check.into(),
            Some(&p.title),
            json!(p.tables.len()),
            json!(r.tables.len()),
            format!("python has {} tables, rust has {}", p.tables.len(), r.tables.len()),
        ));
    }
    for (i, (pt, rt)) in p.tables.iter().zip(&r.tables).enumerate() {
        if wants(Check::Rows) {
            compare_rows(&p.title, i, pt, rt, out);
        }
        if wants(Check::Cursor) && pt.cursor != rt.cursor {
            out.push(Mismatch::cross(
                Kind::Cursor,
                Some(&p.title),
                json!(pt.cursor),
                json!(rt.cursor),
                format!(
                    "table {i}'s cursor is on row {:?} in python and {:?} in rust",
                    pt.cursor, rt.cursor
                ),
            ));
        }
    }
}

/// Compares the Python and Rust structures of one capture. Boxes whose title contains an `ignore.boxes` entry are
/// skipped by every check but `titles`, and a box found in only one app is reported once, under `titles`.
pub fn compare(py: &Structure, rs: &Structure, checks: &[Check], ignore: &Ignore) -> Vec<Mismatch> {
    let ignored = |title: &str| ignore.boxes.iter().any(|i| title.contains(i.as_str()));
    let wants = |c: Check| checks.contains(&c);
    let mut out = Vec::new();

    if wants(Check::Titles) {
        out.extend(compare_titles(py, rs));
    }

    let pairs: Vec<_> = pair_boxes(py, rs)
        .into_iter()
        .filter(|(p, _)| !ignored(&p.title))
        .collect();
    if wants(Check::Tabs) {
        if py.tabs != rs.tabs {
            out.push(Mismatch::cross(
                Kind::Tabs,
                None,
                json!(py.tabs),
                json!(rs.tabs),
                "the screen's tabs differ".into(),
            ));
        }
        for (p, r) in &pairs {
            if p.tabs != r.tabs {
                out.push(Mismatch::cross(
                    Kind::Tabs,
                    Some(&p.title),
                    json!(p.tabs),
                    json!(r.tabs),
                    "tabs differ".into(),
                ));
            }
        }
    }
    for (p, r) in &pairs {
        compare_tables(p, r, &wants, &mut out);
    }
    let (py_footer, rs_footer) = (footer_strings(py), footer_strings(rs));
    let footer_clipped = py.footer_clipped || rs.footer_clipped;
    if wants(Check::Footer) && !clipped_lists_equal(&py_footer, &rs_footer, footer_clipped) {
        let (p, r) = (py_footer, rs_footer);
        out.push(Mismatch::cross(
            Kind::Footer,
            None,
            json!(p),
            json!(r),
            "footer bindings differ".into(),
        ));
    }
    if wants(Check::Focus) {
        let focused = |s: &Structure| -> Vec<String> { s.focused.iter().filter(|t| !ignored(t)).cloned().collect() };
        let (p, r) = (focused(py), focused(rs));
        if p != r {
            out.push(Mismatch::cross(
                Kind::Focus,
                None,
                json!(p),
                json!(r),
                format!("focused boxes are {p:?} in python and {r:?} in rust"),
            ));
        }
    }
    out
}

/// How much of a write's body the `writes` mismatch shows.
const WRITE_BODY_CHARS: usize = 200;

/// One write as the `writes` check keys it. Ordered by its fields, so sets of them list in a stable order.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Write {
    method: String,
    /// The path and its query suffix.
    target: String,
    /// The body as compact JSON, in full.
    body: Option<String>,
}

impl Write {
    /// `METHOD /path?query body`, the body cut at [`WRITE_BODY_CHARS`].
    fn display(&self) -> String {
        let mut line = format!("{} {}", self.method, self.target);
        if let Some(body) = &self.body {
            line.push(' ');
            match body.char_indices().nth(WRITE_BODY_CHARS) {
                Some((cut, _)) => {
                    line.push_str(&body[..cut]);
                    line.push('…');
                }
                None => line.push_str(body),
            }
        }
        line
    }
}

/// The writes in a request log: every line with a method other than `GET`, served, missed or errored alike, except
/// those `ignore.writes` covers. A set, so a write sent twice counts once, as in `requests.jsonl`.
fn writes(log: &str, ignore: &Ignore) -> BTreeSet<Write> {
    RequestLog::parse(&screen::normalize_log(log))
        .entries()
        .iter()
        .filter(|e| !e.is_ssh())
        .filter_map(|e| {
            let method = e.method.as_deref()?;
            let path = e.path.as_deref()?;
            if method.eq_ignore_ascii_case("GET") || ignore.covers_write(method, path) {
                return None;
            }
            let query = e.query.as_ref().map(reference::query_suffix).unwrap_or_default();
            Some(Write {
                method: method.to_owned(),
                target: format!("{path}{query}"),
                body: e.body.as_ref().map(|b| sorted_keys(b).to_string()),
            })
        })
        .collect()
}

/// `value` with every object's keys in sorted order. Python's `json.dumps` and serde write keys in the order the app
/// built the object, which isn't part of what was sent, so the comparison and the message use one order.
fn sorted_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k.clone(), sorted_keys(v))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted_keys).collect()),
        other => other.clone(),
    }
}

/// Compares the writes in two request logs (a `fake-gh.log` or `requests.jsonl`): one `writes` mismatch listing the
/// writes found in only one app, or `None` when both sent the same ones. `ignore` holds `ignore.writes` entries.
pub fn compare_writes(py: &str, rs: &str, ignore: &[String]) -> Option<Mismatch> {
    let ignore = Ignore {
        writes: ignore.to_vec(),
        ..Ignore::default()
    };
    let (p, r) = (writes(py, &ignore), writes(rs, &ignore));
    let only =
        |a: &BTreeSet<Write>, b: &BTreeSet<Write>| -> Vec<String> { a.difference(b).map(Write::display).collect() };
    let (only_py, only_rs) = (only(&p, &r), only(&r, &p));
    let n = only_py.len() + only_rs.len();
    if n == 0 {
        return None;
    }
    let mut parts = Vec::new();
    if !only_py.is_empty() {
        parts.push(format!("only in python: {}", only_py.join(", ")));
    }
    if !only_rs.is_empty() {
        parts.push(format!("only in rust: {}", only_rs.join(", ")));
    }
    let count = if n == 1 {
        "1 write differs".to_owned()
    } else {
        format!("{n} writes differ")
    };
    Some(Mismatch::cross(
        Kind::Writes,
        None,
        json!(only_py),
        json!(only_rs),
        format!("{count}: {}", parts.join("; ")),
    ))
}

/// The `writes` mismatch for one size, with the name of the capture it goes on: the scenario's last, since the
/// writes are the whole run's, not one screen's. Both apps' `fake-gh.log` in `dir` are compared, when both left one.
fn size_writes(scenario: &Scenario, dir: &Path) -> Result<Option<(String, Mismatch)>> {
    let Some(last) = scenario.steps.iter().rev().find_map(|step| match step {
        Step::Capture(c) => Some(c),
        _ => None,
    }) else {
        return Ok(None);
    };
    let mut logs = Vec::new();
    for app in App::ALL {
        let path = dir.join(app.name()).join("fake-gh.log");
        match std::fs::read_to_string(&path) {
            Ok(text) => logs.push(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(HarnessError(format!("reading {}: {e}", path.display()))),
        }
    }
    Ok(match logs.as_slice() {
        [py, rs] => compare_writes(py, rs, &last.ignore.writes).map(|m| (last.name.clone(), m)),
        _ => None,
    })
}

/// Each app's structure of capture `name`, for the apps in `apps` that dumped it, written to its
/// `<capture>.structure.json` with its warnings printed.
fn capture_structures<'a>(
    scenario: &Scenario,
    dir: &Path,
    apps: impl Iterator<Item = &'a App>,
    theme: &ThemeColors,
    name: &str,
) -> Result<BTreeMap<App, Structure>> {
    let size = dir.file_name().unwrap_or_default().to_string_lossy();
    let mut structures = BTreeMap::new();
    for &app in apps {
        let app_dir = dir.join(app.name());
        let grid_file = app_dir.join(format!("{name}.grid.json"));
        if !grid_file.is_file() {
            continue;
        }
        let s = structure::extract(&read_grid(&grid_file)?, theme);
        write_json(&app_dir.join(format!("{name}.structure.json")), &s)?;
        for warning in &s.warnings {
            println!("{} {size} {app} {name}: warning: {warning}", scenario.name);
        }
        structures.insert(app, s);
    }
    Ok(structures)
}

/// The theme name the scenario's config selects: `appearance.theme`, defaulting to the Python app's default.
pub fn scenario_theme(scenario: &Scenario, roots: &Roots) -> Result<String> {
    let file = roots.config_file(&scenario.env.config);
    let base = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let base: Value =
        serde_json::from_str(&base).map_err(|e| HarnessError(format!("{} isn't valid JSON: {e}", file.display())))?;
    let config = build_config(base, &scenario.env.config_overrides);
    Ok(config
        .pointer("/appearance/theme")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_THEME)
        .to_owned())
}

/// The colors `wait.focused` reads in the scenario's theme. A scenario that uses `wait.focused` needs its theme in
/// `parity/themes.toml` with `focus`, `border` and `block_cursor`; otherwise this fails, naming the theme and the
/// missing key, so the run stops before anything starts.
pub fn focus_colors(scenario: &Scenario, themes: &Themes, roots: &Roots) -> Result<FocusColors> {
    let theme = scenario_theme(scenario, roots)?;
    let colors = themes.focus_colors(&theme);
    if !scenario.uses_focused_wait() {
        return Ok(colors);
    }
    if !themes.contains(&theme) {
        return Err(HarnessError(format!(
            "{}: `wait.focused` needs theme {theme:?} in parity/themes.toml, which has no entry for it",
            scenario.path.display()
        )));
    }
    let keys = [
        ("focus", &colors.pane),
        ("border", &colors.border),
        ("block_cursor", &colors.block_cursor),
    ];
    if let Some((key, _)) = keys.iter().find(|(_, color)| color.is_none()) {
        return Err(HarnessError(format!(
            "{}: `wait.focused` needs `{key}` for theme {theme:?} in parity/themes.toml",
            scenario.path.display()
        )));
    }
    Ok(colors)
}

fn read_grid(path: &Path) -> Result<Grid> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let grid: Grid = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if grid.version != GRID_VERSION {
        return Err(HarnessError(format!(
            "{}: grid version {} isn't {GRID_VERSION}",
            path.display(),
            grid.version
        )));
    }
    Ok(grid)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    // Plain data with string keys; serializing can't fail.
    let json = serde_json::to_string_pretty(value).expect("check output serializes");
    std::fs::write(path, json + "\n").with_context(|| format!("writing {}", path.display()))
}

/// The drift on capture `name` of `scenario` at `size`: its console lines and its mismatches for `check.json` (saying
/// which side is which). Its findings for `--ci`, with the hint, go to `findings`.
fn capture_drift(
    scenario: &str,
    size: &str,
    name: &str,
    drift: &[Drift],
    findings: &mut Vec<Finding>,
) -> (Vec<String>, Vec<Mismatch>) {
    let hint = drift_hint(scenario);
    let mut lines = Vec::new();
    let mut mismatches = Vec::new();
    for m in drift.iter().filter(|d| d.capture == name).map(|d| &d.mismatch) {
        lines.push(format!("{scenario} {size} {name}: drift: {}; {hint}", m.describe()));
        let with = |message: String| Mismatch { message, ..m.clone() };
        findings.push(Finding::drift(size, name, &with(format!("{}; {hint}", m.message))));
        mismatches.push(with(format!(
            "{} (python is the live run, rust the reference)",
            m.message
        )));
    }
    (lines, mismatches)
}

/// Runs the structural check for every capture of one scenario at one size. `dir` is
/// `<out>/<scenario>/<COLSxROWS>`, holding one directory per app that ran. Writes `<app>/<capture>.structure.json`
/// for each dump found and `<capture>.check.json`, and prints one line per mismatch. `drift` goes into each
/// capture's `check.json` and counts against Python. Returns each capture's summary and every mismatch with the app
/// it counts against.
pub fn check_size(
    scenario: &Scenario,
    dir: &Path,
    statuses: &BTreeMap<App, AppRunStatus>,
    theme: &ThemeColors,
    drift: &[Drift],
) -> Result<(Vec<CaptureSummary>, Vec<Finding>)> {
    let size = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut summaries = Vec::new();
    let mut findings = Vec::new();
    let mut writes = size_writes(scenario, dir)?;
    for step in &scenario.steps {
        let Step::Capture(capture) = step else { continue };
        let name = &capture.name;
        let structures = capture_structures(scenario, dir, statuses.keys(), theme, name)?;

        let mut lines = Vec::new();
        let mut against = Vec::new();
        let mut expect = Vec::new();
        let expectation = scenario.expect.get(name);
        if let Some(e) = expectation {
            for (&app, s) in &structures {
                let mut found = check_expect(app, s, e);
                if e.opened.is_some() {
                    found.extend(check_opened(app, &opened_by(dir, app, name)?, e));
                }
                if !found.is_empty() {
                    against.push(app);
                }
                lines.extend(found.iter().map(|m| format!("{app} expect {}", m.describe())));
                findings.extend(found.iter().map(|m| Finding::expect(&size, name, app, m)));
                expect.extend(found);
            }
        }
        let both_ok = App::ALL
            .iter()
            .all(|app| statuses.get(app).is_some_and(AppRunStatus::passed));
        let mut cross = Vec::new();
        let cross_ran = match (structures.get(&App::Python), structures.get(&App::Rust)) {
            (Some(py), Some(rs)) if both_ok => {
                cross = compare(py, rs, &capture.checks, &capture.ignore);
                true
            }
            _ => false,
        };
        if let Some((_, m)) = writes.take_if(|(on, _)| on == name) {
            cross.push(m);
        }
        if !cross.is_empty() && !against.contains(&App::Rust) {
            against.push(App::Rust);
        }
        lines.extend(cross.iter().map(|m| format!("cross {}", m.describe())));
        findings.extend(cross.iter().map(|m| Finding::cross(&size, name, m)));

        let (drift_lines, drifted) = capture_drift(&scenario.name, &size, name, drift, &mut findings);
        if !drifted.is_empty() && !against.contains(&App::Python) {
            against.push(App::Python);
        }

        let checked =
            cross_ran || !cross.is_empty() || (expectation.is_some() && !structures.is_empty()) || !drifted.is_empty();
        let mismatches = expect.len() + cross.len() + drifted.len();
        let check = match (checked, mismatches) {
            (false, _) => CheckOutcome::NotApplicable,
            (true, 0) => CheckOutcome::Pass,
            (true, _) => CheckOutcome::Fail,
        };
        // An unchecked capture gets no check.json, so the report says "not checked" rather than "no mismatches".
        if checked {
            let grid_size = structures.values().next().map_or([0, 0], |s| s.size);
            write_json(
                &dir.join(format!("{name}.check.json")),
                &CheckFile {
                    capture: name.clone(),
                    size: grid_size,
                    expect,
                    cross,
                    drift: drifted,
                },
            )?;
        }
        if check != CheckOutcome::NotApplicable {
            println!(
                "{} {size} {name}: check {} ({mismatches} mismatches)",
                scenario.name,
                check.as_str()
            );
            for line in &lines {
                println!("  {line}");
            }
            for line in &drift_lines {
                println!("{line}");
            }
        }
        against.sort();
        summaries.push(CaptureSummary {
            name: name.clone(),
            check,
            mismatches,
            against,
        });
    }
    Ok((summaries, findings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::structure::{Tabs, extract};
    use crate::parity::test_grid;

    fn startup() -> Structure {
        extract(
            &test_grid::load("python/startup-160x48.grid.json"),
            &ThemeColors {
                name: "textual-dark".into(),
                focus: Some("#4ebf71".into()),
            },
        )
    }

    fn all(py: &Structure, rs: &Structure) -> Vec<Mismatch> {
        compare(py, rs, &Check::ALL, &Ignore::default())
    }

    fn kinds(ms: &[Mismatch]) -> Vec<Kind> {
        ms.iter().map(|m| m.check).collect()
    }

    fn pane<'a>(s: &'a mut Structure, title: &str) -> &'a mut Pane {
        s.boxes.iter_mut().find(|b| b.title == title).unwrap()
    }

    fn repos(s: &mut Structure) -> &mut Table {
        &mut pane(s, "[1] Repositories").tables[0]
    }

    /// Renames a box everywhere it's named.
    fn rename(s: &mut Structure, from: &str, to: &str) {
        pane(s, from).title = to.into();
        for f in &mut s.focused {
            if f == from {
                *f = to.into();
            }
        }
    }

    #[test]
    fn identical_structures_match() {
        let s = startup();
        assert_eq!(all(&s, &s.clone()), []);
    }

    #[test]
    fn a_changed_title_is_one_titles_mismatch() {
        let py = startup();
        let mut rs = py.clone();
        rename(&mut rs, "[1] Repositories", "[9] Repositories");
        let ms = all(&py, &rs);
        assert_eq!(kinds(&ms), [Kind::Titles, Kind::Focus], "{ms:#?}");
        assert!(ms[0].message.contains("[9] Repositories"), "{}", ms[0].message);
    }

    #[test]
    fn a_box_in_only_one_app_is_reported_once() {
        let py = startup();
        let mut rs = py.clone();
        rs.boxes.retain(|b| b.title != "[3] Issues");
        let ms = all(&py, &rs);
        assert_eq!(kinds(&ms), [Kind::Titles], "{ms:#?}");
        assert_eq!(ms[0].message, "only in python: \"[3] Issues\"");
    }

    #[test]
    fn a_reordered_title_list_is_detected() {
        let py = startup();
        let mut rs = py.clone();
        rs.boxes.swap(1, 2);
        let ms = all(&py, &rs);
        assert_eq!(kinds(&ms), [Kind::Titles], "{ms:#?}");
        assert!(ms[0].message.contains("different order"));
    }

    #[test]
    fn tab_names_and_the_active_tab_are_compared() {
        let py = startup();
        let mut rs = py.clone();
        pane(&mut rs, "[5] Details").tabs = Some(Tabs {
            names: vec!["Intro".into()],
            active: Some("Introduction".into()),
        });
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Tabs]);
        let mut rs = py.clone();
        pane(&mut rs, "[5] Details").tabs = Some(Tabs {
            names: vec!["Introduction".into()],
            active: None,
        });
        let ms = all(&py, &rs);
        assert_eq!(kinds(&ms), [Kind::Tabs]);
        assert_eq!(ms[0].in_box.as_deref(), Some("[5] Details"));
    }

    #[test]
    fn headers_cells_and_cursor_are_compared() {
        let py = startup();
        let mut rs = py.clone();
        repos(&mut rs).headers[1] = "Login".into();
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Headers]);

        let mut rs = py.clone();
        repos(&mut rs).rows[2][2] = "lazy-demo2".into();
        let ms = all(&py, &rs);
        assert_eq!(kinds(&ms), [Kind::Rows]);
        assert_eq!(ms[0].message, "table 0 row 2 differs");

        let mut rs = py.clone();
        repos(&mut rs).rows.pop();
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Rows]);

        let mut rs = py.clone();
        repos(&mut rs).cursor = Some(1);
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Cursor]);
    }

    #[test]
    fn a_table_missing_in_rust_fails_rows_and_cursor() {
        let py = startup();
        let mut rs = py.clone();
        pane(&mut rs, "[1] Repositories").tables.clear();
        let ms = compare(&py, &rs, &[Check::Rows], &Ignore::default());
        assert_eq!(kinds(&ms), [Kind::Rows], "{ms:#?}");
        assert_eq!(ms[0].in_box.as_deref(), Some("[1] Repositories"));
        assert_eq!(ms[0].message, "python has 1 tables, rust has 0");
        let ms = compare(&py, &rs, &[Check::Cursor], &Ignore::default());
        assert_eq!(kinds(&ms), [Kind::Cursor], "{ms:#?}");
        // One mismatch covers both.
        assert_eq!(
            kinds(&compare(&py, &rs, &[Check::Rows, Check::Cursor], &Ignore::default())),
            [Kind::Rows]
        );
    }

    #[test]
    fn a_clipped_footer_matches_a_longer_one() {
        let py = startup();
        assert!(py.footer_clipped);
        let mut rs = py.clone();
        rs.footer_clipped = false;
        // Rust shows the binding Python cut off in full, and more after it.
        let last = rs.footer.len() - 1;
        rs.footer[last].label.push_str(" and the rest");
        rs.footer.push(Binding {
            key: "z".into(),
            label: "Extra".into(),
        });
        assert_eq!(all(&py, &rs), []);
        // A difference before the cut still counts.
        rs.footer[0].label = "Find".into();
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Footer]);
        // So does a cut shown by neither app.
        let mut py = py;
        py.footer_clipped = false;
        let mut rs = py.clone();
        rs.footer.pop();
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Footer]);
    }

    #[test]
    fn a_clipped_last_header_matches_its_full_text() {
        let mut py = startup();
        let mut rs = py.clone();
        let t = repos(&mut py);
        t.headers_clipped = true;
        t.headers.last_mut().unwrap().truncate(4);
        // Rust's last column is wider than the cut.
        for row in &mut repos(&mut rs).rows {
            row.last_mut().unwrap().push_str(" (more)");
        }
        assert_eq!(all(&py, &rs), []);
        repos(&mut py).headers_clipped = false;
        let rows = repos(&mut py).rows.len();
        let expected: Vec<Kind> = std::iter::once(Kind::Headers)
            .chain(std::iter::repeat_n(Kind::Rows, rows))
            .collect();
        assert_eq!(kinds(&all(&py, &rs)), expected);
        repos(&mut py).headers_clipped = true;
        repos(&mut rs).headers[0] = "Fav".into();
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Headers]);
    }

    #[test]
    fn kind_names_are_the_serde_names() {
        use Kind::*;
        for kind in [
            Titles,
            Tabs,
            Headers,
            Rows,
            Cursor,
            Footer,
            Focus,
            Contains,
            NotContains,
            Requests,
            Writes,
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str(), "{kind:?}");
        }
    }

    #[test]
    fn truncated_cells_match_their_full_text() {
        assert!(cells_equal("hello-wo…", "hello-world"));
        assert!(cells_equal("hello-world ", "hello-wo…"));
        assert!(cells_equal("hel…", "hello-wo…"));
        assert!(!cells_equal("help…", "hello-world"));
        assert!(!cells_equal("hello", "hello-world"));
        let py = startup();
        let mut rs = py.clone();
        repos(&mut rs).rows[1][2] = "hello-w…".into();
        assert_eq!(all(&py, &rs), []);
    }

    #[test]
    fn footer_and_focus_are_compared() {
        let py = startup();
        let mut rs = py.clone();
        rs.footer[0].label = "Find".into();
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Footer]);

        let mut rs = py.clone();
        pane(&mut rs, "[1] Repositories").focused = false;
        rs.focused = vec!["[5] Details".into()];
        assert_eq!(kinds(&all(&py, &rs)), [Kind::Focus]);
    }

    #[test]
    fn only_the_requested_checks_run() {
        let py = startup();
        let mut rs = py.clone();
        rs.footer.clear();
        repos(&mut rs).cursor = None;
        assert_eq!(
            kinds(&compare(&py, &rs, &[Check::Cursor], &Ignore::default())),
            [Kind::Cursor]
        );
        assert_eq!(compare(&py, &rs, &[Check::Titles], &Ignore::default()), []);
    }

    #[test]
    fn ignored_boxes_only_count_for_titles() {
        let py = startup();
        let mut rs = py.clone();
        repos(&mut rs).rows[0][1] = "someone-else".into();
        repos(&mut rs).headers[0] = "Fav".into();
        pane(&mut rs, "[1] Repositories").focused = false;
        rs.focused.clear();
        let ignore = Ignore {
            boxes: vec!["Repositories".into()],
            ..Ignore::default()
        };
        assert_eq!(compare(&py, &rs, &Check::ALL, &ignore), []);
        rename(&mut rs, "[1] Repositories", "[9] Repositories");
        assert_eq!(kinds(&compare(&py, &rs, &Check::ALL, &ignore)), [Kind::Titles]);
    }

    fn startup_expect() -> Expect {
        Expect {
            pane_titles: Some(
                [
                    "",
                    "[1] Repositories",
                    "[5] Details",
                    "[2] Pull Requests",
                    "[3] Issues",
                    "[4] Workflows",
                ]
                .map(String::from)
                .to_vec(),
            ),
            focused: Some(vec!["[1] Repositories".into()]),
            contains: Some(vec!["lazy-demo".into()]),
            not_contains: Some(vec!["Traceback".into()]),
            footer: Some(
                [
                    "/ Search",
                    "^f Toggle Favorite",
                    "O Lookup Repository",
                    "u View User",
                    "I New Issue",
                    "P New PR",
                    "^v Paste link",
                    "f Follow link",
                    "F Follow link (browser)",
                    "q Quit",
                    "^p Command Pallete",
                    "M Maximize",
                    "? Keybindings",
                ]
                .map(String::from)
                .to_vec(),
            ),
            tabs: Some(ExpectTabs {
                in_box: "Details".into(),
                names: Some(vec!["Introduction".into()]),
                active: Some("Introduction".into()),
            }),
            opened: None,
        }
    }

    #[test]
    fn opened_is_the_whole_list_in_order() {
        let e = Expect {
            opened: Some(vec!["https://a".into(), "https://b".into()]),
            ..Expect::default()
        };
        let urls = |urls: &[&str]| urls.iter().map(|u| (*u).to_owned()).collect::<Vec<_>>();
        assert_eq!(check_opened(App::Rust, &urls(&["https://a", "https://b"]), &e), None);
        for wrong in [urls(&[]), urls(&["https://a"]), urls(&["https://b", "https://a"])] {
            let m = check_opened(App::Rust, &wrong, &e).expect("a mismatch");
            assert_eq!(m.check, Kind::Opened);
            assert_eq!(m.rust, json!(wrong));
            assert_eq!(m.python, Value::Null);
        }
        // Without the key nothing is checked.
        assert_eq!(
            check_opened(App::Python, &urls(&["https://x"]), &Expect::default()),
            None
        );
    }

    #[test]
    fn opened_reads_each_apps_browser_log_up_to_the_capture() {
        let dir = tempfile::tempdir().unwrap();
        let rust = dir.path().join(App::Rust.name());
        std::fs::create_dir_all(&rust).unwrap();
        let log = [
            fake_gh::browser::BrowserLine::Opened {
                url: "https://a".into(),
            },
            fake_gh::browser::BrowserLine::Capture { capture: "one".into() },
            fake_gh::browser::BrowserLine::Opened {
                url: "https://b".into(),
            },
        ]
        .iter()
        .map(fake_gh::browser::BrowserLine::to_line)
        .collect::<String>();
        std::fs::write(rust.join(super::super::env::BROWSER_LOG), log).unwrap();
        assert_eq!(opened_by(dir.path(), App::Rust, "one").unwrap(), ["https://a"]);
        // The mark of a capture isn't there yet, or there's no log at all: nothing was opened by then.
        assert!(opened_by(dir.path(), App::Rust, "two").unwrap().is_empty());
        assert!(opened_by(dir.path(), App::Python, "one").unwrap().is_empty());
    }

    #[test]
    fn the_startup_expectation_holds_on_the_python_dump() {
        let ms = check_expect(App::Python, &startup(), &startup_expect());
        assert_eq!(ms, []);
    }

    #[test]
    fn each_expectation_fails_on_its_own() {
        let mut s = startup();
        rename(&mut s, "[1] Repositories", "[9] Repositories");
        s.screen.push("Traceback (most recent call last):".into());
        pane(&mut s, "[5] Details").tabs = None;
        s.footer.truncate(3);
        s.footer_clipped = false;
        let mut e = startup_expect();
        e.contains = Some(vec!["nowhere".into()]);
        let ms = check_expect(App::Rust, &s, &e);
        assert_eq!(
            kinds(&ms),
            [
                Kind::Titles,
                Kind::Focus,
                Kind::Footer,
                Kind::Contains,
                Kind::NotContains,
                Kind::Tabs
            ],
            "{ms:#?}"
        );
        assert!(ms[0].message.contains("[9] Repositories"));
        assert_eq!(ms[0].python, Value::Null);
        assert_eq!(ms[0].rust[1], "[9] Repositories");
        assert_eq!(ms[0].expected.as_ref().unwrap()[1], "[1] Repositories");
        let s = startup();
        let focus = check_expect(
            App::Python,
            &s,
            &Expect {
                focused: Some(vec!["[5] Details".into()]),
                ..Expect::default()
            },
        );
        assert_eq!(kinds(&focus), [Kind::Focus]);
    }

    #[test]
    fn a_clipped_footer_matches_a_prefix_of_the_expected_one() {
        let expected: Vec<String> = ["q Quit", "^p Command Pallete", "? Keys"].map(String::from).to_vec();
        let shown = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(footer_matches(&shown(&["q Quit", "^p Comman"]), true, &expected));
        assert!(footer_matches(
            &shown(&["q Quit", "^p Command Pallete", "? Keys"]),
            true,
            &expected
        ));
        assert!(!footer_matches(&shown(&["q Quit", "^p Comman"]), false, &expected));
        assert!(!footer_matches(&shown(&["q Quit", "^x Comman"]), true, &expected));
        assert!(!footer_matches(&shown(&["q Exit", "^p Comman"]), true, &expected));
    }

    /// A scenario `drifty` whose capture checks `rows`, and a live grid whose table differs from the stored one.
    #[test]
    fn a_focused_wait_needs_its_themes_focus_colors() {
        let repo = crate::parity::cli::repo_root();
        let roots = Roots {
            fixtures: repo.join("parity/fixtures"),
            configs: repo.join("parity/configs"),
            seeds: repo.join("parity/seeds"),
        };
        let themes = Themes::parse(
            "[textual-dark]\nfocus = \"#4ebf71\"\nborder = \"#0178d4\"\nblock_cursor = \"#0178d4\"\n\
             [partial]\nfocus = \"#4ebf71\"\nborder = \"#0178d4\"\n",
        )
        .unwrap();
        let scenario = |theme: &str, wait: &str| {
            let source = format!(
                "version = 1\n[env]\nconfig_overrides = {{ appearance = {{ theme = \"{theme}\" }} }}\n\
                 [[step]]\nwait = {wait}\n"
            );
            Scenario::parse(Path::new("parity/scenarios/focusy.toml"), &source, &roots).unwrap()
        };
        let focused = "{ focused = \"Save\" }";

        let colors = focus_colors(&scenario("textual-dark", focused), &themes, &roots).unwrap();
        assert_eq!(colors.border.as_deref(), Some("#0178d4"));

        let missing = focus_colors(&scenario("nord", focused), &themes, &roots)
            .unwrap_err()
            .to_string();
        assert!(missing.contains("parity/scenarios/focusy.toml"), "{missing}");
        assert!(missing.contains("theme \"nord\" in parity/themes.toml"), "{missing}");

        let partial = focus_colors(&scenario("partial", focused), &themes, &roots)
            .unwrap_err()
            .to_string();
        assert!(
            partial.contains("needs `block_cursor` for theme \"partial\""),
            "{partial}"
        );

        // A scenario without the wait runs on any theme.
        let plain = focus_colors(&scenario("nord", "{ text = \"x\" }"), &themes, &roots).unwrap();
        assert_eq!(plain, FocusColors::default());
    }

    fn drifted_table() -> (Scenario, ThemeColors, Grid, Grid) {
        let repo = crate::parity::cli::repo_root();
        let roots = Roots {
            fixtures: repo.join("parity/fixtures"),
            configs: repo.join("parity/configs"),
            seeds: repo.join("parity/seeds"),
        };
        let source = "version = 1\ngate = false\n[[step]]\ncapture = \"main\"\nchecks = [\"rows\"]\n";
        let scenario = Scenario::parse(Path::new("drifty.toml"), source, &roots).unwrap();
        let theme = ThemeColors {
            name: "textual-dark".into(),
            focus: Some("#4ebf71".into()),
        };
        let stored = test_grid::load("table.txt");
        let mut live = stored.clone();
        // "beta" becomes "bets" in the live run: table 0, row 1.
        let row = live.lines.iter_mut().find(|l| l.text().contains("beta")).unwrap();
        let col = row.cells.windows(4).position(|w| w.concat() == "beta").unwrap();
        row.cells[col + 3] = "s".into();
        (scenario, theme, live, stored)
    }

    fn side(label: &str, grid: &Grid, requests: &str) -> crate::parity::reference::Side {
        crate::parity::reference::Side {
            label: label.into(),
            failure: None,
            captures: vec![("main".into(), grid.clone())],
            requests: requests.into(),
            browser: String::new(),
        }
    }

    #[test]
    fn drift_is_what_a_live_run_disagrees_with_its_reference_on() {
        use crate::parity::reference::{self, LIVE_LABEL, REFERENCE_LABEL};

        let (scenario, theme, live, stored) = drifted_table();
        let served = r#"{"seq":0,"argv":["api","/user"],"command":"api","method":"GET","path":"/user","query":{},"accept":"json","cache":null,"cache_control":null,"body":null,"fixture":"base/user/GET.json","status":200,"miss":false,"error":null}"#;
        let drift = reference::drift(
            &scenario,
            &theme,
            &side(LIVE_LABEL, &live, &format!("{served}\n")),
            &side(REFERENCE_LABEL, &stored, ""),
        );
        let kinds: Vec<Kind> = drift.iter().map(|d| d.mismatch.check).collect();
        assert_eq!(kinds, [Kind::Rows, Kind::Requests], "{drift:#?}");
        assert!(drift.iter().all(|d| d.capture == "main"), "{drift:#?}");
        assert_eq!(drift[0].mismatch.message, "table 0 row 1 differs");
        assert_eq!(
            drift[1].mismatch.message,
            "the served requests differ; only in the live run: GET /user (json)"
        );
        // A failed live run isn't compared: its failure is reported already.
        let mut failed = side(LIVE_LABEL, &live, "");
        failed.failure = Some("timeout at step 0 (1.0s)".into());
        assert!(reference::drift(&scenario, &theme, &failed, &side(REFERENCE_LABEL, &stored, "")).is_empty());
    }

    #[test]
    fn drift_fails_the_scenario_against_python() {
        use crate::parity::reference::{self, LIVE_LABEL, REFERENCE_LABEL};
        use crate::parity::runner::{AppRunStatus, RunState};
        use crate::parity::summary::{ScenarioResult, ScenarioSummary, SizeSummary};

        let (scenario, theme, live, stored) = drifted_table();
        let drift = reference::drift(
            &scenario,
            &theme,
            &side(LIVE_LABEL, &live, ""),
            &side(REFERENCE_LABEL, &stored, ""),
        );
        // The Rust side matches the live run, so the only mismatches are drift.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("44x10");
        for (app, grid) in [(App::Python, &live), (App::Rust, &live)] {
            let app_dir = dir.join(app.name());
            std::fs::create_dir_all(&app_dir).unwrap();
            std::fs::write(app_dir.join("main.grid.json"), grid.to_json()).unwrap();
        }
        let ok: AppRunStatus = serde_json::from_value(json!({
            "status": RunState::Ok, "step": null, "exit_code": null, "duration_ms": 1, "flaky": false,
            "fixture_problems": false, "misses": [], "errors": []
        }))
        .unwrap();
        let statuses = BTreeMap::from([(App::Python, ok.clone()), (App::Rust, ok)]);
        let (captures, findings) = check_size(&scenario, &dir, &statuses, &theme, &drift).unwrap();

        let check: CheckFile =
            serde_json::from_str(&std::fs::read_to_string(dir.join("main.check.json")).unwrap()).unwrap();
        assert!(check.expect.is_empty() && check.cross.is_empty(), "{check:#?}");
        assert_eq!(check.drift.len(), 1, "{check:#?}");
        let m = &check.drift[0];
        assert_eq!((m.check, m.in_box.as_deref()), (Kind::Rows, Some("[1] Repositories")));
        assert_eq!(m.python[1], "bets");
        assert_eq!(m.rust[1], "beta");
        assert_eq!(
            m.message,
            "table 0 row 1 differs (python is the live run, rust the reference)"
        );
        assert_eq!(captures[0].against, [App::Python]);
        assert_eq!((captures[0].check, captures[0].mismatches), (CheckOutcome::Fail, 1));

        let summary = ScenarioSummary {
            name: "drifty".into(),
            description: String::new(),
            gate: false,
            parity_rows: Vec::new(),
            result: ScenarioResult::from_statuses(
                false,
                &[SizeSummary {
                    size: "44x10".into(),
                    apps: statuses,
                    captures,
                }],
            ),
            sizes: Vec::new(),
            error: None,
        };
        // Drift fails even an ungated scenario: it's against Python.
        assert_eq!(summary.result, ScenarioResult::Fail);
        let annotations: Vec<String> =
            crate::parity::ci::annotations("parity/scenarios/drifty.toml", &summary, &findings)
                .iter()
                .map(ToString::to_string)
                .collect();
        assert_eq!(
            annotations,
            [
                "::error file=parity/scenarios/drifty.toml::drifty/44x10/main: drift rows [1] Repositories: table 0 row \
                 1 differs; the reference is wrong or nondeterministic: run cargo xtask parity --record --force \
                 drifty and review the diff"
            ]
        );
    }

    /// A fake-gh log line for `method path` with `body`, served unless `miss`.
    fn logged(method: &str, path: &str, body: &Value, miss: bool) -> String {
        let entry = json!({
            "seq": 1, "argv": ["api", path], "command": "api", "method": method, "path": path, "query": {},
            "accept": "json", "cache": null, "cache_control": null, "body": body,
            "fixture": if miss { Value::Null } else { json!("base/x.json") }, "status": if miss { 404 } else { 200 },
            "miss": miss, "error": null
        });
        format!("{entry}\n")
    }

    fn log(lines: &[String]) -> String {
        lines.concat()
    }

    const ISSUES: &str = "/repos/o/r/issues";

    #[test]
    fn equal_writes_compare_equal_and_gets_are_ignored() {
        let post = logged("POST", ISSUES, &json!({"title": "x"}), false);
        let py = log(&[logged("GET", "/user", &Value::Null, false), post.clone()]);
        let rs = log(&[post, logged("GET", "/user/repos", &Value::Null, false)]);
        assert_eq!(compare_writes(&py, &rs, &[]), None);
        assert_eq!(compare_writes("", "", &[]), None);
    }

    #[test]
    fn body_key_order_doesnt_count() {
        let body = |text: &str| serde_json::from_str::<Value>(text).unwrap();
        let py = logged("POST", "/x", &body(r#"{"title":"x","labels":[{"b":1,"a":2}]}"#), false);
        let rs = logged("POST", "/x", &body(r#"{"labels":[{"a":2,"b":1}],"title":"x"}"#), false);
        assert_eq!(compare_writes(&py, &rs, &[]), None);
        let m = compare_writes(&py, "", &[]).unwrap();
        assert_eq!(m.python, json!([r#"POST /x {"labels":[{"a":2,"b":1}],"title":"x"}"#]));
    }

    #[test]
    fn a_write_in_only_one_app_is_listed_under_it() {
        let post = logged("POST", ISSUES, &json!({"title": "x", "body": "y"}), false);
        let m = compare_writes(&post, "", &[]).unwrap();
        assert_eq!(m.check, Kind::Writes);
        assert_eq!(m.in_box, None);
        assert_eq!(m.python, json!([r#"POST /repos/o/r/issues {"body":"y","title":"x"}"#]));
        assert_eq!(m.rust, json!([]));
        assert_eq!(
            m.message,
            r#"1 write differs: only in python: POST /repos/o/r/issues {"body":"y","title":"x"}"#
        );

        let m = compare_writes("", &logged("DELETE", "/repos/o/r/labels/bug", &Value::Null, false), &[]).unwrap();
        assert_eq!(m.python, json!([]));
        assert_eq!(m.rust, json!(["DELETE /repos/o/r/labels/bug"]));
        assert_eq!(m.message, "1 write differs: only in rust: DELETE /repos/o/r/labels/bug");
    }

    #[test]
    fn different_bodies_to_one_path_are_both_listed() {
        let py = logged("PATCH", "/repos/o/r/issues/1", &json!({"state": "closed"}), false);
        let rs = logged("PATCH", "/repos/o/r/issues/1", &json!({"state": "open"}), false);
        let m = compare_writes(&py, &rs, &[]).unwrap();
        assert_eq!(
            m.message,
            r#"2 writes differ: only in python: PATCH /repos/o/r/issues/1 {"state":"closed"}; only in rust: PATCH /repos/o/r/issues/1 {"state":"open"}"#
        );
    }

    #[test]
    fn a_missed_write_is_still_compared() {
        let served = logged("POST", ISSUES, &json!({"title": "x"}), false);
        let missed = logged("POST", ISSUES, &json!({"title": "x"}), true);
        assert_eq!(compare_writes(&served, &missed, &[]), None);
        let m = compare_writes("", &missed, &[]).unwrap();
        assert_eq!(m.rust, json!([r#"POST /repos/o/r/issues {"title":"x"}"#]));
    }

    #[test]
    fn duplicate_writes_collapse() {
        let post = logged("POST", ISSUES, &json!({"title": "x"}), false);
        let mut twice = post.replace("\"seq\":1", "\"seq\":2");
        twice.insert_str(0, &post);
        assert_eq!(compare_writes(&post, &twice, &[]), None);
    }

    #[test]
    fn the_query_is_part_of_the_key() {
        let with_query = |q: Value| {
            let mut line: Value = serde_json::from_str(&logged("PUT", "/x", &Value::Null, false)).unwrap();
            line["query"] = q;
            format!("{line}\n")
        };
        let m = compare_writes(&with_query(json!({"a": "1"})), &with_query(json!({})), &[]).unwrap();
        assert_eq!(m.python, json!(["PUT /x?a=1"]));
        assert_eq!(m.rust, json!(["PUT /x"]));
    }

    #[test]
    fn ignored_writes_match_by_method_and_path() {
        let merge = logged(
            "PUT",
            "/repos/o/r/pulls/1/merge",
            &json!({"merge_method": "squash"}),
            false,
        );
        let ignore = ["PUT /repos/o/r/pulls/1/merge".to_owned()];
        assert_eq!(compare_writes(&merge, "", &ignore), None);
        assert!(compare_writes(&merge, "", &["POST /repos/o/r/pulls/1/merge".to_owned()]).is_some());
        assert!(compare_writes(&merge, "", &["PUT /repos/o/r/pulls/2/merge".to_owned()]).is_some());
    }

    #[test]
    fn long_bodies_are_cut() {
        let text = "a".repeat(300);
        let m = compare_writes(&logged("POST", ISSUES, &json!({ "body": text }), false), "", &[]).unwrap();
        let line = m.python[0].as_str().unwrap();
        let body = line.strip_prefix("POST /repos/o/r/issues ").unwrap();
        assert_eq!(body.chars().count(), WRITE_BODY_CHARS + 1, "{line}");
        assert!(body.starts_with(r#"{"body":"aaa"#) && body.ends_with("a…"), "{line}");
        // Bodies that differ only past the cut still differ.
        let other = format!("{}b", "a".repeat(299));
        let m = compare_writes(
            &logged("POST", ISSUES, &json!({ "body": text }), false),
            &logged("POST", ISSUES, &json!({ "body": other }), false),
            &[],
        )
        .unwrap();
        assert_eq!(m.message.matches("…").count(), 2, "{}", m.message);
    }

    #[test]
    fn mismatches_serialize_in_the_check_json_shape() {
        let m = Mismatch::cross(Kind::NotContains, Some("A"), json!(1), Value::Null, "m".into());
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({"check": "not_contains", "box": "A", "python": 1, "rust": null, "message": "m"})
        );
    }
}
