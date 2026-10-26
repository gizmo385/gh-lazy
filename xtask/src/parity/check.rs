//! The structural check: each capture's [`Structure`] against the scenario's `[expect.<capture>]` (per app), and
//! Python against Rust (cross-app). Results go to `<capture>.check.json` and feed `summary.json`.
//!
//! [`check_expect`] and [`compare`] are pure; [`check_size`] reads the grid dumps, extracts, checks and writes.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ci::Finding;
use super::env::build_config;
use super::grid::{GRID_VERSION, Grid};
use super::runner::AppRunStatus;
use super::scenario::{App, Check, Expect, ExpectTabs, Ignore, Roots, Scenario, Step};
use super::structure::{self, Binding, DEFAULT_THEME, Pane, Structure, Table, ThemeColors};
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
    for step in &scenario.steps {
        let Step::Capture(capture) = step else { continue };
        let name = &capture.name;
        let mut structures: BTreeMap<App, Structure> = BTreeMap::new();
        for &app in statuses.keys() {
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

        let mut lines = Vec::new();
        let mut against = Vec::new();
        let mut expect = Vec::new();
        let expectation = scenario.expect.get(name);
        if let Some(e) = expectation {
            for (&app, s) in &structures {
                let found = check_expect(app, s, e);
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
        if !cross.is_empty() && !against.contains(&App::Rust) {
            against.push(App::Rust);
        }
        lines.extend(cross.iter().map(|m| format!("cross {}", m.describe())));
        findings.extend(cross.iter().map(|m| Finding::cross(&size, name, m)));

        let (drift_lines, drifted) = capture_drift(&scenario.name, &size, name, drift, &mut findings);
        if !drifted.is_empty() && !against.contains(&App::Python) {
            against.push(App::Python);
        }

        let checked = cross_ran || (expectation.is_some() && !structures.is_empty()) || !drifted.is_empty();
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
        }
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
    fn drifted_table() -> (Scenario, ThemeColors, Grid, Grid) {
        let repo = crate::parity::cli::repo_root();
        let roots = Roots {
            fixtures: repo.join("parity/fixtures"),
            configs: repo.join("parity/configs"),
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

    #[test]
    fn mismatches_serialize_in_the_check_json_shape() {
        let m = Mismatch::cross(Kind::NotContains, Some("A"), json!(1), Value::Null, "m".into());
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({"check": "not_contains", "box": "A", "python": 1, "rust": null, "message": "m"})
        );
    }
}
