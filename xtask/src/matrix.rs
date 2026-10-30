//! `cargo xtask parity-matrix`: parses the parity matrix in `docs/rust-rewrite/parity/`, checks it and the scenarios
//! and golden manifests that name its rows, and prints the number of rows per area and status, then per area and
//! coverage. The layout and editing rules are in `docs/rust-rewrite/parity/README.md`; golden manifests are described
//! in `parity/README.md`, "Python goldens".
//!
//! The check also runs in `cargo test` (`xtask/tests/parity_matrix.rs`), so a malformed matrix fails CI.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

/// The matrix directory, relative to the repository root.
pub const MATRIX_DIR: &str = "docs/rust-rewrite/parity";
/// The scenarios whose `parity_rows` must name rows, relative to the repository root.
pub const SCENARIOS_DIR: &str = "parity/scenarios";
/// The golden manifests, relative to the repository root.
pub const GOLDENS_DIR: &str = "parity/goldens";

/// The values a row's Status may start with. `intentional diff` may be followed by ` (<qualifier>)`.
pub const STATUSES: [&str; 4] = ["not started", "in progress", "parity", "intentional diff"];
/// The milestones a row's Milestone may name (PLAN §8), in order.
pub const MILESTONES: [&str; 7] = ["M0", "M1", "M2", "M3", "M4", "M5", "M6"];
/// The [`Matrix::counts_by_milestone`] entry for status rows without a valid milestone.
pub const NO_MILESTONE: &str = "none";
const INTENTIONAL_DIFF: &str = "intentional diff";
const PY_BUG_STATUSES: [&str; 3] = ["open", "answered", "withdrawn"];

static ROW_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([a-z0-9]+(?:-[a-z0-9]+)*)\.[a-z0-9]+(?:-[a-z0-9]+)*$").expect("valid regex"));
static SLUG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").expect("valid regex"));
static MILESTONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^M[0-6]$").expect("valid regex"));
static ISSUES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#\d+(?:, #\d+)*$").expect("valid regex"));
static BUG_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bB(\d+)\b").expect("valid regex"));
static BUG_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^B(\d{2,})\.md$").expect("valid regex"));
static DECISION_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\(\.\./decisions/([^)]*)\.md\)").expect("valid regex"));
static FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^- ([A-Z][A-Za-z ]*): (.*)$").expect("valid regex"));
static CODE_SPAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]*)`").expect("valid regex"));
static INDEX_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\]\(\./areas/([^)]*)\.md\)").expect("valid regex"));

/// A problem in the matrix, located by file (relative to the repository root) and 1-based line (0 = whole file).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Problem {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file.display(), self.message)
        } else {
            write!(f, "{}:{}: {}", self.file.display(), self.line, self.message)
        }
    }
}

/// One row of an area table.
#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub line: usize,
    /// `None` for tables without a Status column (terminal caveats).
    pub status: Option<String>,
    /// The Milestone cell as written (`M0`–`M6` when the matrix is valid), or `None` for tables without one.
    pub milestone: Option<String>,
    /// The whole table line, for finding `Py bug` notes and decision links in any cell.
    pub source: String,
}

impl Row {
    /// The status without a `(qualifier)`, e.g. `intentional diff` for `intentional diff (gh only)`.
    pub fn base_status(&self) -> Option<&str> {
        self.status.as_deref().map(|s| s.split(" (").next().unwrap_or(s))
    }
}

/// One `areas/<name>.md` file.
#[derive(Debug, Clone)]
pub struct Area {
    pub name: String,
    pub file: PathBuf,
    pub rows: Vec<Row>,
}

/// The scenario and golden manifest files (relative to the repository root) whose `parity_rows` name a row.
#[derive(Debug, Clone, Default)]
pub struct Covered {
    pub scenarios: Vec<PathBuf>,
    pub goldens: Vec<PathBuf>,
}

/// The coverage table's columns after Rows, in order.
pub const COVERAGE: [&str; 4] = ["Scenario", "Golden", "Both", "Neither"];
const NEITHER: usize = 3;

impl Covered {
    /// The index into [`COVERAGE`] this row counts under.
    fn column(&self) -> usize {
        match (self.scenarios.is_empty(), self.goldens.is_empty()) {
            (false, true) => 0,
            (true, false) => 1,
            (false, false) => 2,
            (true, true) => NEITHER,
        }
    }
}

/// A `parity/goldens/<name>.toml` manifest: a Python-dumped golden, the Rust test that checks it, and the rows it
/// covers.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenManifest {
    version: u32,
    // Only people read it; it is required so every manifest says what it checks.
    #[expect(dead_code, reason = "read by people, not by the check")]
    description: String,
    generator: PathBuf,
    golden: Vec<PathBuf>,
    test: PathBuf,
    fresh_test: String,
    parity_rows: Vec<String>,
}

/// The parsed matrix plus everything the check found wrong while parsing and cross-checking it.
#[derive(Debug, Default)]
pub struct Matrix {
    /// In README index order; area files missing from the index come last, by name.
    pub areas: Vec<Area>,
    pub py_bugs: BTreeSet<u32>,
    pub decisions: BTreeSet<String>,
    /// For each row id that a scenario or golden manifest names, which ones name it. Rows nothing names are absent.
    pub coverage: BTreeMap<String, Covered>,
    pub problems: Vec<Problem>,
}

impl Matrix {
    pub fn row_ids(&self) -> impl Iterator<Item = &str> {
        self.areas.iter().flat_map(|a| a.rows.iter().map(|r| r.id.as_str()))
    }

    /// Rows per status for each area that has a Status column, in area order.
    pub fn counts(&self) -> Vec<(&str, [usize; STATUSES.len()])> {
        self.status_areas()
            .map(|a| {
                let mut counts = [0; STATUSES.len()];
                for status in a.rows.iter().filter_map(Row::base_status) {
                    if let Some(i) = STATUSES.iter().position(|s| *s == status) {
                        counts[i] += 1;
                    }
                }
                (a.name.as_str(), counts)
            })
            .collect()
    }

    /// Rows per status for each milestone, M0 to M6, across every area with a Status column. All seven milestones are
    /// listed, even with no rows. Status rows without a valid Milestone (a matrix problem) are counted under a last
    /// [`NO_MILESTONE`] entry, present only when there are any, so the totals match [`Matrix::counts`].
    pub fn counts_by_milestone(&self) -> Vec<(&str, [usize; STATUSES.len()])> {
        let mut counts: Vec<(&str, [usize; STATUSES.len()])> =
            MILESTONES.iter().map(|m| (*m, [0; STATUSES.len()])).collect();
        let mut none = [0; STATUSES.len()];
        for row in self.status_areas().flat_map(|a| &a.rows) {
            let Some(status) = row.base_status().and_then(|s| STATUSES.iter().position(|x| *x == s)) else {
                continue;
            };
            match row
                .milestone
                .as_deref()
                .and_then(|m| MILESTONES.iter().position(|x| *x == m))
            {
                Some(i) => counts[i].1[status] += 1,
                None => none[status] += 1,
            }
        }
        if none.iter().any(|n| *n > 0) {
            counts.push((NO_MILESTONE, none));
        }
        counts
    }

    /// The counts as an aligned text table, or as a Markdown table.
    pub fn summary(&self, markdown: bool) -> String {
        let counts = self.counts();
        let mut header = vec!["Area"];
        header.extend(STATUSES);
        header.push("Rows");
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut totals = [0; STATUSES.len()];
        for (area, c) in &counts {
            let mut row = vec![(*area).to_owned()];
            row.extend(c.iter().map(ToString::to_string));
            row.push(c.iter().sum::<usize>().to_string());
            rows.push(row);
            for (t, n) in totals.iter_mut().zip(c) {
                *t += n;
            }
        }
        let mut total = vec!["Total".to_owned()];
        total.extend(totals.iter().map(ToString::to_string));
        total.push(totals.iter().sum::<usize>().to_string());
        rows.push(total);

        render_table(&header, &rows, markdown)
    }

    /// For each area with a Status column, in area order: its row count, then its rows per coverage column
    /// (Scenario, Golden, Both, Neither). The four add up to the row count.
    pub fn coverage_counts(&self) -> Vec<(&str, [usize; 1 + COVERAGE.len()])> {
        self.status_areas()
            .map(|a| {
                let mut counts = [0; 1 + COVERAGE.len()];
                for row in &a.rows {
                    counts[0] += 1;
                    counts[1 + self.coverage.get(&row.id).map_or(NEITHER, Covered::column)] += 1;
                }
                (a.name.as_str(), counts)
            })
            .collect()
    }

    /// The coverage counts as an aligned text table, or as a Markdown table.
    pub fn coverage_summary(&self, markdown: bool) -> String {
        let mut header = vec!["Area", "Rows"];
        header.extend(COVERAGE);
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut totals = [0; 1 + COVERAGE.len()];
        for (area, c) in self.coverage_counts() {
            let mut row = vec![area.to_owned()];
            row.extend(c.iter().map(ToString::to_string));
            rows.push(row);
            for (t, n) in totals.iter_mut().zip(c) {
                *t += n;
            }
        }
        let mut total = vec!["Total".to_owned()];
        total.extend(totals.iter().map(ToString::to_string));
        rows.push(total);
        render_table(&header, &rows, markdown)
    }

    /// The rows no scenario or golden names, in area order, except intentional diffs: the backlog of rows that still
    /// need a check.
    pub fn uncovered(&self) -> Vec<&Row> {
        self.status_areas()
            .flat_map(|a| &a.rows)
            .filter(|r| !self.coverage.contains_key(&r.id) && r.base_status() != Some(INTENTIONAL_DIFF))
            .collect()
    }

    /// [`Matrix::uncovered`] as `<id>  <status>` lines, for `--uncovered`.
    pub fn uncovered_report(&self) -> String {
        let mut out = String::new();
        for row in self.uncovered() {
            let _ = writeln!(out, "{}  {}", row.id, row.status.as_deref().unwrap_or_default());
        }
        out
    }

    /// The areas that have a Status column (terminal caveats don't).
    fn status_areas(&self) -> impl Iterator<Item = &Area> {
        self.areas.iter().filter(|a| a.rows.iter().any(|r| r.status.is_some()))
    }
}

/// `rows` under `header` as an aligned text table (the first column left-aligned, the rest right-aligned), or as a
/// Markdown table.
fn render_table(header: &[&str], rows: &[Vec<String>], markdown: bool) -> String {
    let mut out = String::new();
    if markdown {
        let _ = writeln!(out, "| {} |", header.join(" | "));
        let _ = writeln!(out, "|{}", " --- |".repeat(header.len()));
        for row in rows {
            let _ = writeln!(out, "| {} |", row.join(" | "));
        }
    } else {
        let widths: Vec<usize> = (0..header.len())
            .map(|i| {
                rows.iter()
                    .map(|r| r[i].len())
                    .chain([header[i].len()])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let line = |cells: &[&str]| {
            let mut s = format!("{:<w$}", cells[0], w = widths[0]);
            for (cell, w) in cells.iter().zip(&widths).skip(1) {
                let _ = write!(s, "  {cell:>w$}");
            }
            s.trim_end().to_owned()
        };
        let _ = writeln!(out, "{}", line(header));
        for row in rows {
            let cells: Vec<&str> = row.iter().map(String::as_str).collect();
            let _ = writeln!(out, "{}", line(&cells));
        }
    }
    out
}

/// Parses and checks the matrix under `repo`, the `parity_rows` of every scenario, and every golden manifest.
pub fn check(repo: &Path) -> Matrix {
    let mut m = Matrix::default();
    let dir = repo.join(MATRIX_DIR);
    let rel = |p: &Path| p.strip_prefix(repo).unwrap_or(p).to_owned();

    load_areas(&dir, &rel, &mut m);
    check_unique_ids(&mut m);
    let ids: BTreeSet<String> = m.row_ids().map(str::to_owned).collect();
    check_py_bugs(&dir, &rel, &ids, &mut m);
    check_decisions(&dir, &rel, &ids, &mut m);
    check_row_references(&mut m);
    check_scenarios(repo, &ids, &mut m);
    check_goldens(repo, &ids, &mut m);

    m.problems.sort();
    m
}

type Rel<'a> = dyn Fn(&Path) -> PathBuf + 'a;

/// The area names the README's `## Index` section links, in order.
fn index_names(readme: &str) -> Vec<String> {
    let section = readme.split_once("\n## Index\n").map_or("", |(_, s)| s);
    let section = section.split("\n## ").next().unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for c in INDEX_LINK.captures_iter(section) {
        if !names.iter().any(|n| *n == c[1]) {
            names.push(c[1].to_owned());
        }
    }
    names
}

/// Parses the area files in README index order; files missing from the index come last, by name.
fn load_areas(dir: &Path, rel: &Rel<'_>, m: &mut Matrix) {
    let readme = dir.join("README.md");
    let index = match fs::read_to_string(&readme) {
        Ok(text) => index_names(&text),
        Err(e) => {
            m.problems.push(problem(&rel(&readme), 0, format!("can't read: {e}")));
            Vec::new()
        }
    };
    let area_files = md_files(&dir.join("areas"), rel, &mut m.problems);
    for name in &index {
        if !area_files.contains_key(name) {
            m.problems.push(problem(
                &rel(&readme),
                0,
                format!("the index links areas/{name}.md, which doesn't exist"),
            ));
        }
    }
    let mut ordered: Vec<&String> = index.iter().filter(|n| area_files.contains_key(*n)).collect();
    for name in area_files.keys() {
        if !index.contains(name) {
            m.problems.push(problem(
                &rel(&readme),
                0,
                format!("areas/{name}.md is missing from the index"),
            ));
            ordered.push(name);
        }
    }
    for name in ordered {
        let path = &area_files[name];
        match fs::read_to_string(path) {
            Ok(text) => m.areas.push(parse_area(name, &rel(path), &text, &mut m.problems)),
            Err(e) => m.problems.push(problem(&rel(path), 0, format!("can't read: {e}"))),
        }
    }
}

fn check_unique_ids(m: &mut Matrix) {
    let mut seen: BTreeMap<&str, (&Path, usize)> = BTreeMap::new();
    let mut duplicates = Vec::new();
    for area in &m.areas {
        for row in &area.rows {
            if let Some((file, line)) = seen.insert(&row.id, (&area.file, row.line)) {
                duplicates.push(problem(
                    &area.file,
                    row.line,
                    format!("duplicate row id `{}` (also at {}:{line})", row.id, file.display()),
                ));
            }
        }
    }
    m.problems.extend(duplicates);
}

fn check_py_bugs(dir: &Path, rel: &Rel<'_>, ids: &BTreeSet<String>, m: &mut Matrix) {
    for (stem, path) in md_files(&dir.join("py-bugs"), rel, &mut m.problems) {
        let file = rel(&path);
        let Some(number) = BUG_FILE
            .captures(&format!("{stem}.md"))
            .and_then(|c| c[1].parse::<u32>().ok())
        else {
            m.problems.push(problem(
                &file,
                0,
                "py-bug files are named B<nn>.md, e.g. B07.md".to_owned(),
            ));
            continue;
        };
        if !m.py_bugs.insert(number) {
            m.problems
                .push(problem(&file, 0, format!("duplicate py-bug number B{number}")));
        }
        check_record(&path, &file, ids, &mut m.problems, |fields, problems| {
            check_field_eq(fields, "Id", &format!("B{number}"), &file, problems);
            match fields.get("Status") {
                Some((line, status)) if !PY_BUG_STATUSES.contains(&status.as_str()) => problems.push(problem(
                    &file,
                    *line,
                    format!("status `{status}` isn't one of {}", PY_BUG_STATUSES.join(", ")),
                )),
                Some((line, status)) if status == "answered" && !fields.contains_key("Answer") => {
                    problems.push(problem(
                        &file,
                        *line,
                        "an answered py bug needs an `Answer:` line".to_owned(),
                    ));
                }
                _ => {}
            }
            &["Id", "Status", "Python reference", "Recommendation", "Rows"]
        });
    }
}

fn check_decisions(dir: &Path, rel: &Rel<'_>, ids: &BTreeSet<String>, m: &mut Matrix) {
    for (stem, path) in md_files(&dir.join("decisions"), rel, &mut m.problems) {
        let file = rel(&path);
        if !SLUG.is_match(&stem) {
            m.problems.push(problem(
                &file,
                0,
                "decision files are named <kebab-case-slug>.md".to_owned(),
            ));
        }
        m.decisions.insert(stem);
        check_record(&path, &file, ids, &mut m.problems, |_, _| {
            &["Decision", "Decided by", "Link", "Rows"]
        });
    }
}

/// Every `Py bug Bn` has a file, every decision link resolves, and every intentional diff links a decision.
fn check_row_references(m: &mut Matrix) {
    let mut refs = Vec::new();
    for area in &m.areas {
        for row in &area.rows {
            let line = row.source.as_str();
            if line.contains("Py bug") {
                for c in BUG_REF.captures_iter(line) {
                    let n: u32 = c[1].parse().unwrap_or(0);
                    if !m.py_bugs.contains(&n) {
                        refs.push(problem(
                            &area.file,
                            row.line,
                            format!("`{}` notes Py bug B{n}, but py-bugs/B{n:02}.md doesn't exist", row.id),
                        ));
                    }
                }
            }
            let linked: Vec<&str> = DECISION_LINK
                .captures_iter(line)
                .map(|c| c.get(1).map_or("", |g| g.as_str()))
                .collect();
            for slug in &linked {
                if !m.decisions.contains(*slug) {
                    refs.push(problem(
                        &area.file,
                        row.line,
                        format!("`{}` links decisions/{slug}.md, which doesn't exist", row.id),
                    ));
                }
            }
            if row.base_status() == Some(INTENTIONAL_DIFF) && linked.is_empty() {
                refs.push(problem(
                    &area.file,
                    row.line,
                    format!(
                        "`{}` is an intentional diff but its Notes don't link a decision \
                         (`Decision: [<slug>](../decisions/<slug>.md)`)",
                        row.id
                    ),
                ));
            }
        }
    }
    m.problems.extend(refs);
}

fn problem(file: &Path, line: usize, message: String) -> Problem {
    Problem {
        file: file.to_owned(),
        line,
        message,
    }
}

/// The `*.md` files in `dir` by stem. A missing directory is a problem.
fn md_files(dir: &Path, rel: &Rel<'_>, problems: &mut Vec<Problem>) -> BTreeMap<String, PathBuf> {
    let mut files = BTreeMap::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    files.insert(stem.to_owned(), path.clone());
                }
            }
        }
        Err(e) => problems.push(problem(&rel(dir), 0, format!("can't read directory: {e}"))),
    }
    files
}

/// Splits a table row into trimmed cells. `\|` is a literal pipe inside a cell.
fn split_row(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push_str("\\|");
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cell).trim().to_owned()),
            _ => cell.push(c),
        }
    }
    cells.push(cell.trim().to_owned());
    cells
}

fn parse_area(name: &str, file: &Path, text: &str, problems: &mut Vec<Problem>) -> Area {
    let mut area = Area {
        name: name.to_owned(),
        file: file.to_owned(),
        rows: Vec::new(),
    };
    if !SLUG.is_match(name) {
        problems.push(problem(file, 0, "area files are named <kebab-case-name>.md".to_owned()));
    }
    let mut header: Option<Vec<String>> = None;
    let mut in_table = false;
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if !line.starts_with('|') {
            in_table = false;
            header = None;
            continue;
        }
        let cells = split_row(line);
        if !in_table {
            in_table = true;
            if cells.first().map(String::as_str) != Some("Id") {
                problems.push(problem(
                    file,
                    n,
                    "every table in an area file starts with an Id column".to_owned(),
                ));
            }
            header = Some(cells);
            continue;
        }
        let Some(header) = header.as_ref() else { continue };
        if cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        if header.first().map(String::as_str) != Some("Id") {
            continue;
        }
        if cells.len() != header.len() {
            problems.push(problem(
                file,
                n,
                format!("{} cells, but the header has {}", cells.len(), header.len()),
            ));
            continue;
        }
        let col = |name: &str| header.iter().position(|h| h == name).map(|i| cells[i].as_str());
        let id = cells[0].trim_matches('`').to_owned();
        match ROW_ID.captures(&id) {
            None => problems.push(problem(
                file,
                n,
                format!("row id `{id}` isn't `<area>.<name>` in kebab-case"),
            )),
            Some(c) if &c[1] != name => problems.push(problem(
                file,
                n,
                format!("row id `{id}` must start with this file's area, `{name}.`"),
            )),
            Some(_) => {}
        }
        let row = Row {
            id,
            line: n,
            status: col("Status").map(str::to_owned),
            milestone: col("Milestone").map(str::to_owned),
            source: line.to_owned(),
        };
        if let Some(status) = &row.status {
            let base = row.base_status().unwrap_or_default();
            let qualified = base != status.as_str();
            if !STATUSES.contains(&base) || (qualified && (base != INTENTIONAL_DIFF || !status.ends_with(')'))) {
                problems.push(problem(
                    file,
                    n,
                    format!(
                        "status `{status}` isn't one of {} (only `{INTENTIONAL_DIFF}` takes a `(qualifier)`)",
                        STATUSES.map(|s| format!("`{s}`")).join(", ")
                    ),
                ));
            }
        }
        if let Some(milestone) = col("Milestone")
            && !MILESTONE.is_match(milestone)
        {
            problems.push(problem(file, n, format!("milestone `{milestone}` isn't M0–M6")));
        }
        if let Some(issue) = col("Issue")
            && !issue.is_empty()
            && !ISSUES.is_match(issue)
        {
            problems.push(problem(file, n, format!("issue `{issue}` isn't `#n` or `#n, #m`")));
        }
        area.rows.push(row);
    }
    area
}

type Fields = BTreeMap<String, (usize, String)>;

/// Reads a py-bug or decision file's `- Key: value` lines, checks the required ones exist and every id on its
/// `Rows:` line is a row, then runs `extra`, which returns the required keys.
fn check_record(
    path: &Path,
    file: &Path,
    ids: &BTreeSet<String>,
    problems: &mut Vec<Problem>,
    extra: impl FnOnce(&Fields, &mut Vec<Problem>) -> &'static [&'static str],
) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            problems.push(problem(file, 0, format!("can't read: {e}")));
            return;
        }
    };
    let mut fields = Fields::new();
    for (i, line) in text.lines().enumerate() {
        if let Some(c) = FIELD.captures(line) {
            fields.entry(c[1].to_owned()).or_insert((i + 1, c[2].trim().to_owned()));
        }
    }
    if !text.starts_with("# ") {
        problems.push(problem(file, 1, "starts with a `# ` heading".to_owned()));
    }
    for key in extra(&fields, problems) {
        if !fields.contains_key(*key) {
            problems.push(problem(file, 0, format!("missing the `- {key}: …` line")));
        }
    }
    if let Some((line, rows)) = fields.get("Rows") {
        let named: Vec<&str> = CODE_SPAN
            .captures_iter(rows)
            .map(|c| c.get(1).map_or("", |g| g.as_str()))
            .collect();
        if named.is_empty() {
            problems.push(problem(
                file,
                *line,
                "`Rows:` lists no row ids (write them as `area.name`)".to_owned(),
            ));
        }
        for id in named {
            if !ids.contains(id) {
                problems.push(problem(file, *line, format!("unknown row id `{id}`")));
            }
        }
    }
}

fn check_field_eq(fields: &Fields, key: &str, want: &str, file: &Path, problems: &mut Vec<Problem>) {
    if let Some((line, value)) = fields.get(key)
        && value != want
    {
        problems.push(problem(
            file,
            *line,
            format!("`{key}: {value}` doesn't match the file name ({want})"),
        ));
    }
}

/// The `*.toml` files in `repo/dir`, sorted, each as (path relative to `repo`, full path). A missing directory has
/// none.
fn toml_files(repo: &Path, dir: &str) -> Vec<(PathBuf, PathBuf)> {
    let Ok(entries) = fs::read_dir(repo.join(dir)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| (path.strip_prefix(repo).unwrap_or(&path).to_owned(), path))
        .collect()
}

/// Checks that each of `rows`, the `parity_rows` of `file` (whose contents are `text`), is a row id, and returns the
/// ones that are. Scenarios and golden manifests share it, so their messages and line lookup match.
fn check_parity_rows<'a>(
    file: &Path,
    text: &str,
    rows: impl IntoIterator<Item = &'a str>,
    ids: &BTreeSet<String>,
    problems: &mut Vec<Problem>,
) -> Vec<&'a str> {
    let mut known = Vec::new();
    for row in rows {
        if ids.contains(row) {
            known.push(row);
        } else {
            let line = text
                .lines()
                .position(|l| l.contains(&format!("\"{row}\"")))
                .map_or(0, |i| i + 1);
            problems.push(problem(
                file,
                line,
                format!("parity_rows names `{row}`, which isn't a row id in {MATRIX_DIR}/areas/"),
            ));
        }
    }
    known
}

fn check_scenarios(repo: &Path, ids: &BTreeSet<String>, m: &mut Matrix) {
    for (file, path) in toml_files(repo, SCENARIOS_DIR) {
        let Ok(text) = fs::read_to_string(&path) else { continue };
        // Invalid scenario files are `cargo xtask parity --validate`'s business.
        let Ok(table) = text.parse::<toml::Table>() else {
            continue;
        };
        let Some(rows) = table.get("parity_rows").and_then(toml::Value::as_array) else {
            continue;
        };
        let rows = rows.iter().filter_map(toml::Value::as_str);
        for row in check_parity_rows(&file, &text, rows, ids, &mut m.problems) {
            m.coverage
                .entry(row.to_owned())
                .or_default()
                .scenarios
                .push(file.clone());
        }
    }
}

/// The 1-based line of the `key = …` line in a TOML file, or 0 if there isn't one.
fn key_line(text: &str, key: &str) -> usize {
    text.lines()
        .position(|l| {
            l.strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        })
        .map_or(0, |i| i + 1)
}

/// Checks every golden manifest: it parses, its paths exist, its `test` and `fresh_test` are set up as
/// `parity/README.md` says, and its `parity_rows` are row ids.
fn check_goldens(repo: &Path, ids: &BTreeSet<String>, m: &mut Matrix) {
    for (file, path) in toml_files(repo, GOLDENS_DIR) {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                m.problems.push(problem(&file, 0, format!("can't read: {e}")));
                continue;
            }
        };
        let manifest: GoldenManifest = match toml::from_str(&text) {
            Ok(manifest) => manifest,
            Err(e) => {
                let line = e.span().map_or(0, |s| {
                    text.get(..s.start).map_or(0, |before| before.matches('\n').count() + 1)
                });
                m.problems.push(problem(&file, line, e.message().trim_end().to_owned()));
                continue;
            }
        };
        if manifest.version != 1 {
            m.problems.push(problem(
                &file,
                key_line(&text, "version"),
                format!("version {} isn't supported (only 1 is)", manifest.version),
            ));
        }
        let rows = manifest.parity_rows.iter().map(String::as_str);
        for row in check_parity_rows(&file, &text, rows, ids, &mut m.problems) {
            m.coverage.entry(row.to_owned()).or_default().goldens.push(file.clone());
        }
        let paths = [("generator", &manifest.generator), ("test", &manifest.test)]
            .into_iter()
            .chain(manifest.golden.iter().map(|g| ("golden", g)));
        for (key, rel) in paths {
            if !repo.join(rel).is_file() {
                m.problems.push(problem(
                    &file,
                    key_line(&text, key),
                    format!("{key} file `{}` doesn't exist", rel.display()),
                ));
            }
        }
        // A missing `test` file is reported above.
        if let Ok(source) = fs::read_to_string(repo.join(&manifest.test)) {
            check_golden_test(&file, &text, &manifest, &source, &mut m.problems);
        }
    }
}

/// A `#[test]` fn that [`test_fns`] found.
struct TestFn {
    name: String,
    ignored: bool,
}

/// The `#[test]` fns in a Rust source file. This is a line scan, not a Rust parser: a line starting with `#[test]`
/// (after trimming) starts an attribute block, which ends at the next line starting with `fn ` or `async fn `, and the
/// fn is ignored if a line in the block starts with `#[ignore`. So `#[tokio::test]`, `#[rstest]`, `pub fn` and
/// attributes sharing a line aren't seen. The golden tests don't need them.
fn test_fns(source: &str) -> Vec<TestFn> {
    let mut fns = Vec::new();
    // `Some(ignored)` inside an attribute block.
    let mut block: Option<bool> = None;
    for line in source.lines().map(str::trim) {
        if line.starts_with("#[test]") {
            block = Some(false);
        } else if let Some(ignored) = block {
            if line.starts_with("#[ignore") {
                block = Some(true);
            } else if let Some(rest) = line.strip_prefix("fn ").or_else(|| line.strip_prefix("async fn ")) {
                let name = rest.split(['(', '<']).next().unwrap_or_default().trim().to_owned();
                fns.push(TestFn { name, ignored });
                block = None;
            }
        }
    }
    fns
}

/// The manifest's `test` file has a comparison that runs in plain `cargo test`, and its `fresh_test`, `#[ignore]`d.
fn check_golden_test(file: &Path, text: &str, manifest: &GoldenManifest, source: &str, problems: &mut Vec<Problem>) {
    let fns = test_fns(source);
    let test_file = manifest.test.display();
    if !fns.iter().any(|f| !f.ignored) {
        problems.push(problem(
            file,
            key_line(text, "test"),
            format!("{test_file} has no #[test] that isn't #[ignore]d, so plain `cargo test` doesn't check the golden"),
        ));
    }
    let fresh = &manifest.fresh_test;
    match fns.iter().find(|f| f.name == *fresh) {
        None => problems.push(problem(
            file,
            key_line(text, "fresh_test"),
            format!("fresh_test `{fresh}` isn't a #[test] fn in {test_file}"),
        )),
        Some(f) if !f.ignored => problems.push(problem(
            file,
            key_line(text, "fresh_test"),
            format!(
                "fresh_test `{fresh}` in {test_file} isn't #[ignore]d; it needs Python, so it must only run with \
                 --ignored"
            ),
        )),
        Some(_) => {}
    }
}

/// `cargo xtask parity-matrix`: prints the problems and exits 1, or prints the status and coverage tables (with
/// `uncovered`, the rows nothing covers instead).
pub fn run(repo: &Path, markdown: bool, uncovered: bool) -> std::process::ExitCode {
    let matrix = check(repo);
    if matrix.problems.is_empty() {
        if uncovered {
            print!("{}", matrix.uncovered_report());
        } else {
            print!("{}\n{}", matrix.summary(markdown), matrix.coverage_summary(markdown));
        }
        std::process::ExitCode::SUCCESS
    } else {
        for p in &matrix.problems {
            eprintln!("{p}");
        }
        eprintln!(
            "{} problem(s) in the parity matrix (rules: {MATRIX_DIR}/README.md)",
            matrix.problems.len()
        );
        std::process::ExitCode::FAILURE
    }
}

/// The area file that holds a row id, relative to the repository root, e.g.
/// `docs/rust-rewrite/parity/areas/main-layout.md` for `main-layout.footer`. `None` if `id` isn't a row id.
pub fn row_file(id: &str) -> Option<String> {
    let area = ROW_ID.captures(id)?.get(1)?.as_str().to_owned();
    Some(format!("{MATRIX_DIR}/areas/{area}.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_rows_with_escaped_pipes_and_empty_cells() {
        assert_eq!(
            split_row("| `a.b` | x \\| y | | z |"),
            vec!["`a.b`", "x \\| y", "", "z"]
        );
    }

    #[test]
    fn finds_test_fns_and_their_ignore_attributes() {
        let source = "\
#[test]
fn plain() {}

    #[cfg(unix)]
    #[test]
    #[ignore = \"needs Python\"]
    async fn ignored<T>() {}

#[rstest]
fn not_seen() {}

fn helper() {}
";
        let fns: Vec<(String, bool)> = test_fns(source).into_iter().map(|f| (f.name, f.ignored)).collect();
        assert_eq!(fns, [("plain".to_owned(), false), ("ignored".to_owned(), true)]);
    }

    #[test]
    fn row_file_names_the_area_file() {
        assert_eq!(
            row_file("main-layout.footer").as_deref(),
            Some("docs/rust-rewrite/parity/areas/main-layout.md")
        );
        assert_eq!(row_file("Main layout"), None);
    }
}
