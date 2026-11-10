//! The check of a run: each capture's [`Structure`] against the scenario's `[expect.<capture>]`, and the snapshot
//! comparison's mismatches ([`super::snapshot::check_run`]). Results go to `<capture>.check.json` and feed
//! `summary.json`.
//!
//! [`check_expect`] and [`check_opened`] are pure; [`check_size`] reads the grid dumps, extracts, checks and writes.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ci::Finding;
use super::grid::{GRID_VERSION, Grid};
use super::scenario::{Expect, ExpectTabs, Scenario, Step};
use super::snapshot::SnapshotMismatch;
use super::structure::{self, Binding, Structure, ThemeColors};
use super::summary::{CaptureSummary, CheckOutcome};
use super::{Context, HarnessError, Result};

/// What a mismatch is about: an `[expect]` key, or the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Titles,
    Tabs,
    Footer,
    Focus,
    Contains,
    NotContains,
    /// The URLs the app had opened in the browser by a capture (`[expect] opened`).
    Opened,
    /// A capture's masked screen against its committed snapshot (`parity/snapshots/`).
    Snapshot,
    /// What the run did outside the screen against its snapshot's `effects.txt` (on the last capture).
    Effects,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Titles => "titles",
            Kind::Tabs => "tabs",
            Kind::Footer => "footer",
            Kind::Focus => "focus",
            Kind::Contains => "contains",
            Kind::NotContains => "not_contains",
            Kind::Opened => "opened",
            Kind::Snapshot => "snapshot",
            Kind::Effects => "effects",
        }
    }
}

/// One failed check. For an `[expect]` failure, `actual` holds what the screen showed and `expected` the expectation;
/// a snapshot mismatch has neither, and its message says what differs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mismatch {
    pub check: Kind,
    /// The title of the box the mismatch is in, when it's about one box.
    #[serde(rename = "box")]
    pub in_box: Option<String>,
    pub actual: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    pub message: String,
}

impl Mismatch {
    fn expect(check: Kind, in_box: Option<&str>, actual: Value, expected: Value, message: String) -> Self {
        Self {
            check,
            in_box: in_box.map(str::to_owned),
            actual,
            expected: Some(expected),
            message,
        }
    }

    /// A run against its snapshot ([`Kind::Snapshot`] or [`Kind::Effects`]): the message says what differs.
    pub fn snapshot(check: Kind, message: String) -> Self {
        Self {
            check,
            in_box: None,
            actual: Value::Null,
            expected: None,
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
    /// Where the run differs from its committed snapshot (`snapshot` and `effects`). Left out when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshot: Vec<Mismatch>,
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

/// Checks a capture's structure against `[expect.<capture>]`.
pub fn check_expect(s: &Structure, e: &Expect) -> Vec<Mismatch> {
    let mut out = Vec::new();
    if let Some(expected) = &e.pane_titles {
        let actual = titles(s);
        if actual != *expected {
            out.push(Mismatch::expect(
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
                Kind::NotContains,
                None,
                json!(row.trim()),
                json!(needle),
                format!("a row contains {needle:?}: {:?}", row.trim()),
            ));
        }
    }
    out.extend(e.tabs.as_ref().and_then(|t| expect_tabs(s, t)));
    out
}

/// Checks what the app had opened in the browser by a capture (`opened`, from fake-browser's log) against
/// `[expect.<capture>] opened`.
pub fn check_opened(opened: &[String], e: &Expect) -> Option<Mismatch> {
    let expected = e.opened.as_ref()?;
    (opened != expected.as_slice()).then(|| {
        Mismatch::expect(
            Kind::Opened,
            None,
            json!(opened),
            json!(expected),
            format!("opened {opened:?}, expected {expected:?}"),
        )
    })
}

/// What the app had opened in the browser by capture `name` in the run dir `dir`, from its `browser.jsonl`. A run
/// without the log opened nothing.
fn opened_by(dir: &Path, name: &str) -> Result<Vec<String>> {
    let path = dir.join(super::env::BROWSER_LOG);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(fake_gh::browser::opened_by(&text, name)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(HarnessError(format!("reading {}: {e}", path.display()))),
    }
}

fn expect_tabs(s: &Structure, t: &ExpectTabs) -> Option<Mismatch> {
    let expected = json!({"names": t.names, "active": t.active});
    let Some(b) = s.boxes.iter().find(|b| b.title.contains(t.in_box.as_str())) else {
        return Some(Mismatch::expect(
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
        Kind::Tabs,
        Some(&b.title),
        json!(b.tabs),
        expected,
        message,
    ))
}

/// The structure of capture `name` when the run in `dir` dumped it, written to `<capture>.structure.json` with its
/// warnings printed.
fn capture_structure(scenario: &Scenario, dir: &Path, theme: &ThemeColors, name: &str) -> Result<Option<Structure>> {
    let grid_file = dir.join(format!("{name}.grid.json"));
    if !grid_file.is_file() {
        return Ok(None);
    }
    let s = structure::extract(&read_grid(&grid_file)?, theme);
    write_json(&dir.join(format!("{name}.structure.json")), &s)?;
    let size = dir.file_name().unwrap_or_default().to_string_lossy();
    for warning in &s.warnings {
        println!("{} {size} {name}: warning: {warning}", scenario.name);
    }
    Ok(Some(s))
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

/// The snapshot mismatches on capture `name` of `scenario` at `size`: their console lines and the mismatches. Their
/// findings for `--ci` go to `findings`.
fn capture_snapshot(
    scenario: &str,
    size: &str,
    name: &str,
    snapshot: &[SnapshotMismatch],
    findings: &mut Vec<Finding>,
) -> (Vec<String>, Vec<Mismatch>) {
    let mut lines = Vec::new();
    let mut mismatches = Vec::new();
    for m in snapshot.iter().filter(|m| m.capture == name) {
        lines.push(match m.mismatch.check {
            Kind::Effects => format!("{scenario} {size}: {}", m.mismatch.describe()),
            _ => format!("{scenario} {size} {name}: {}", m.mismatch.describe()),
        });
        findings.push(Finding::snapshot(size, name, &m.mismatch));
        mismatches.push(m.mismatch.clone());
    }
    (lines, mismatches)
}

/// Checks every capture of one scenario at one size. `dir` is the run's directory, `<out>/<scenario>/<COLSxROWS>`.
/// Writes `<capture>.structure.json` for each dump found and `<capture>.check.json` for each capture checked, and
/// prints one line per mismatch. `snapshot` holds the run's snapshot mismatches when its snapshot was compared
/// (`None` when it wasn't); they go into `check.json` too. Returns each capture's summary and every mismatch as a
/// finding.
pub fn check_size(
    scenario: &Scenario,
    dir: &Path,
    theme: &ThemeColors,
    snapshot: Option<&[SnapshotMismatch]>,
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
        let structure = capture_structure(scenario, dir, theme, name)?;

        let mut expect = Vec::new();
        let expectation = scenario.expect.get(name);
        if let (Some(e), Some(s)) = (expectation, &structure) {
            expect = check_expect(s, e);
            if e.opened.is_some() {
                expect.extend(check_opened(&opened_by(dir, name)?, e));
            }
            findings.extend(expect.iter().map(|m| Finding::expect(&size, name, m)));
        }
        let (snapshot_lines, snapshotted) =
            capture_snapshot(&scenario.name, &size, name, snapshot.unwrap_or_default(), &mut findings);
        // A capture compared with its snapshot was checked, whatever else ran.
        let checked = structure.is_some() && (expectation.is_some() || snapshot.is_some());
        let mismatches = expect.len() + snapshotted.len();
        let check = match (checked, mismatches) {
            (false, _) => CheckOutcome::NotApplicable,
            (true, 0) => CheckOutcome::Pass,
            (true, _) => CheckOutcome::Fail,
        };
        // An unchecked capture gets no check.json, so the report says "not checked" rather than "no mismatches".
        if checked {
            println!(
                "{} {size} {name}: check {} ({mismatches} mismatches)",
                scenario.name,
                check.as_str()
            );
            for m in &expect {
                println!("  expect {}", m.describe());
            }
            for line in &snapshot_lines {
                println!("{line}");
            }
            write_json(
                &dir.join(format!("{name}.check.json")),
                &CheckFile {
                    capture: name.clone(),
                    size: structure.as_ref().map_or([0, 0], |s| s.size),
                    expect,
                    snapshot: snapshotted,
                },
            )?;
        }
        summaries.push(CaptureSummary {
            name: name.clone(),
            check,
            mismatches,
        });
    }
    Ok((summaries, findings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::structure::{Pane, extract};
    use crate::parity::test_grid;

    /// The Python app's startup screen, kept as a frozen test vector (`xtask/tests/grids/python/`).
    fn startup() -> Structure {
        extract(
            &test_grid::load("python/startup-160x48.grid.json"),
            &ThemeColors {
                name: "textual-dark".into(),
                focus: Some("#4ebf71".into()),
            },
        )
    }

    fn kinds(ms: &[Mismatch]) -> Vec<Kind> {
        ms.iter().map(|m| m.check).collect()
    }

    fn pane<'a>(s: &'a mut Structure, title: &str) -> &'a mut Pane {
        s.boxes.iter_mut().find(|b| b.title == title).unwrap()
    }

    fn rename(s: &mut Structure, from: &str, to: &str) {
        pane(s, from).title = to.into();
        for f in &mut s.focused {
            if f == from {
                *f = to.into();
            }
        }
    }

    #[test]
    fn kind_names_are_the_serde_names() {
        use Kind::*;
        for kind in [
            Titles,
            Tabs,
            Footer,
            Focus,
            Contains,
            NotContains,
            Opened,
            Snapshot,
            Effects,
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str(), "{kind:?}");
        }
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
        assert_eq!(check_opened(&urls(&["https://a", "https://b"]), &e), None);
        for wrong in [urls(&[]), urls(&["https://a"]), urls(&["https://b", "https://a"])] {
            let m = check_opened(&wrong, &e).expect("a mismatch");
            assert_eq!(m.check, Kind::Opened);
            assert_eq!(m.actual, json!(wrong));
        }
        // Without the key nothing is checked.
        assert_eq!(check_opened(&urls(&["https://x"]), &Expect::default()), None);
    }

    #[test]
    fn opened_reads_the_browser_log_up_to_the_capture() {
        let dir = tempfile::tempdir().unwrap();
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
        assert!(
            opened_by(dir.path(), "one").unwrap().is_empty(),
            "no log, nothing opened"
        );
        std::fs::write(dir.path().join(super::super::env::BROWSER_LOG), log).unwrap();
        assert_eq!(opened_by(dir.path(), "one").unwrap(), ["https://a"]);
        // The mark of a capture isn't there yet: nothing was opened by then.
        assert!(opened_by(dir.path(), "two").unwrap().is_empty());
    }

    #[test]
    fn the_startup_expectation_holds_on_the_startup_dump() {
        assert_eq!(check_expect(&startup(), &startup_expect()), []);
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
        let ms = check_expect(&s, &e);
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
        assert_eq!(ms[0].actual[1], "[9] Repositories");
        assert_eq!(ms[0].expected.as_ref().unwrap()[1], "[1] Repositories");
        let focus = check_expect(
            &startup(),
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

    #[test]
    fn mismatches_serialize_in_the_check_json_shape() {
        let m = Mismatch::expect(Kind::NotContains, Some("A"), json!(1), json!("x"), "m".into());
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({"check": "not_contains", "box": "A", "actual": 1, "expected": "x", "message": "m"})
        );
        let m = Mismatch::snapshot(Kind::Snapshot, "rows 1-1 differ".into());
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({"check": "snapshot", "box": null, "actual": null, "message": "rows 1-1 differ"})
        );
    }
}
