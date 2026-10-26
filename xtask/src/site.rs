//! `cargo xtask parity-site`: writes the landing page of the published parity site
//! (<https://www.gizmo.codes/gh-lazy/parity/>). The page is one self-contained `index.html` showing the commit and
//! time of the parity run (from the report's `meta.json`), its results (from `summary.json`), and the parity matrix's
//! status by milestone and by area and its coverage. The Pages workflow (`.github/workflows/pages.yaml`) puts the
//! report itself under `report/` next to it; this task doesn't copy it.
//!
//! A matrix with problems doesn't stop the page: the problems are listed on it, so a broken matrix is visible rather
//! than blocking the deploy. A report without `index.html` does stop it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::matrix::{self, COVERAGE, MATRIX_DIR, Matrix, STATUSES};
use crate::parity::report::{self, RunMeta, badge, escape};
use crate::parity::summary::{ScenarioResult, Summary};
use crate::parity::{Context, HarnessError};

/// The repository the links point at when `GITHUB_REPOSITORY` isn't set (a local run).
pub const DEFAULT_REPO_URL: &str = "https://github.com/gizmo385/gh-lazy";
/// The branch the matrix and PLAN links point at.
const BRANCH: &str = "rust-main";
/// Where `cargo xtask parity --all` writes its report, relative to the repository root.
const DEFAULT_REPORT: &str = "parity/reports/latest";
/// The landing page's link to the report, relative to the page: the workflow copies the report there.
const REPORT_LINK: &str = "report/index.html";

/// Writing to a `String` can't fail.
macro_rules! w {
    ($dst:expr, $($arg:tt)*) => {{
        let _ = write!($dst, $($arg)*);
    }};
}

#[derive(Debug, clap::Args)]
pub struct SiteArgs {
    /// The report directory a `cargo xtask parity` run wrote [default: parity/reports/latest in the repository].
    #[arg(long)]
    pub report: Option<PathBuf>,
    /// The directory to write the landing page's index.html into. Created if missing.
    #[arg(long)]
    pub out: PathBuf,
}

/// Why the landing page couldn't be written.
#[derive(Debug, thiserror::Error)]
pub enum SiteError {
    /// The report directory has no `index.html`: the parity run didn't get as far as writing its report.
    #[error("{} has no index.html; the parity run wrote no report", .0.display())]
    MissingReport(PathBuf),
    /// Reading the report's files or writing the page failed.
    #[error(transparent)]
    Io(#[from] HarnessError),
}

/// Everything the landing page shows.
#[derive(Debug)]
pub struct Site {
    pub meta: RunMeta,
    pub summary: Summary,
    pub matrix: Matrix,
    /// `https://github.com/<owner>/<repo>`, for the commit, matrix and PLAN links.
    pub repo_url: String,
}

/// The repository URL from `GITHUB_SERVER_URL` and `GITHUB_REPOSITORY` (as Actions sets them), or
/// [`DEFAULT_REPO_URL`] when `repository` is unset or empty.
pub fn repo_url(server: Option<&str>, repository: Option<&str>) -> String {
    match repository.map(str::trim).filter(|r| !r.is_empty()) {
        Some(repository) => {
            let server = server
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("https://github.com");
            format!("{}/{repository}", server.trim_end_matches('/'))
        }
        None => DEFAULT_REPO_URL.to_owned(),
    }
}

/// Loads what the page shows from the report in `report_dir` and the matrix under `repo`.
pub fn load(repo: &Path, report_dir: &Path, repo_url: String) -> Result<Site, SiteError> {
    if !report_dir.join("index.html").is_file() {
        return Err(SiteError::MissingReport(report_dir.to_owned()));
    }
    let meta = RunMeta::read(report_dir)?;
    let path = report_dir.join("summary.json");
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let summary = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Site {
        meta,
        summary,
        matrix: matrix::check(repo),
        repo_url,
    })
}

/// `cargo xtask parity-site`: writes `<out>/index.html`. Exit 1 when the report is missing or a file can't be read
/// or written; matrix problems are printed as warnings and listed on the page.
pub fn run(repo: &Path, args: &SiteArgs) -> ExitCode {
    let report_dir = args.report.clone().unwrap_or_else(|| repo.join(DEFAULT_REPORT));
    let url = repo_url(
        std::env::var("GITHUB_SERVER_URL").ok().as_deref(),
        std::env::var("GITHUB_REPOSITORY").ok().as_deref(),
    );
    match write(repo, &report_dir, &args.out, url) {
        Ok((path, site)) => {
            for problem in &site.matrix.problems {
                eprintln!("parity-site: warning: {problem}");
            }
            println!("parity-site: wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("parity-site: {e}");
            ExitCode::FAILURE
        }
    }
}

fn write(repo: &Path, report_dir: &Path, out: &Path, url: String) -> Result<(PathBuf, Site), SiteError> {
    let site = load(repo, report_dir, url)?;
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let path = out.join("index.html");
    std::fs::write(&path, site.render()).with_context(|| format!("writing {}", path.display()))?;
    Ok((path, site))
}

impl Site {
    /// The landing page.
    pub fn render(&self) -> String {
        let mut body = String::from("<h1>LazyGithub Rust rewrite: parity</h1>\n");
        body.push_str(&self.render_build());
        body.push_str(&self.render_results());
        body.push_str(&self.render_matrix());
        format!(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
             <title>LazyGithub parity</title>\n<style>\n{}{SITE_CSS}</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
            report::PAGE_CSS
        )
    }

    fn blob(&self, path: &str) -> String {
        format!("{}/blob/{BRANCH}/{path}", self.repo_url)
    }

    /// The commit (short sha linked, plus its subject) and the time of the run.
    fn render_build(&self) -> String {
        let sha = &self.meta.git_sha;
        let commit = if !sha.is_empty() && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            format!(
                "<a href=\"{}/commit/{}\"><code>{}</code></a>",
                escape(&self.repo_url),
                escape(sha),
                escape(&sha[..sha.len().min(7)])
            )
        } else {
            format!("<code>{}</code>", escape(sha))
        };
        let subject = if self.meta.subject.is_empty() {
            String::new()
        } else {
            format!(" {}", escape(&self.meta.subject))
        };
        format!(
            "<table>\n<tr><th>Commit</th><td>{commit}{subject}</td></tr>\n\
             <tr><th>Built</th><td>{}</td></tr>\n</table>\n",
            escape(&self.meta.timestamp)
        )
    }

    /// The run's results, the failures by name, and the link to the full report.
    fn render_results(&self) -> String {
        let scenarios = &self.summary.scenarios;
        let n = |r: ScenarioResult| scenarios.iter().filter(|s| s.result == r).count();
        let mut html = String::from("<h2>Latest parity run</h2>\n<p>");
        let mut results = vec![ScenarioResult::Pass, ScenarioResult::Fail, ScenarioResult::RustNotReady];
        results.extend(
            [ScenarioResult::PythonOnly, ScenarioResult::RustOnly]
                .into_iter()
                .filter(|r| n(*r) > 0),
        );
        let parts: Vec<String> = results
            .iter()
            .map(|r| format!("<strong>{}</strong> {}", n(*r), badge(r.as_str())))
            .collect();
        w!(
            html,
            "{} of {} scenario(s). {}</p>\n",
            parts.join(" "),
            scenarios.len(),
            escape(&self.meta.python_line())
        );
        w!(
            html,
            "<p><a href=\"{REPORT_LINK}\">Full report</a>: every scenario at every size, side by side.</p>\n"
        );
        let failed: Vec<_> = scenarios.iter().filter(|s| s.result == ScenarioResult::Fail).collect();
        if !failed.is_empty() {
            html.push_str("<p>Failed:</p>\n<ul>\n");
            for s in failed {
                w!(
                    html,
                    "<li><a href=\"report/{0}/report.html\">{0}</a>{1}</li>\n",
                    escape(&s.name),
                    s.error
                        .as_deref()
                        .map_or_else(String::new, |e| format!(": {}", escape(e)))
                );
            }
            html.push_str("</ul>\n");
        }
        html
    }

    /// The matrix's problems, then its status by milestone and by area, its coverage, and the links.
    fn render_matrix(&self) -> String {
        let m = &self.matrix;
        let mut html = String::from("<h2>Parity matrix</h2>\n");
        w!(
            html,
            "<p><a href=\"{}\">The matrix</a> lists every user-visible behavior of the Python app and its state in the \
             rewrite; <a href=\"{}\">PLAN §6.3</a> describes how parity is checked.</p>\n",
            escape(&format!("{}/tree/{BRANCH}/{MATRIX_DIR}", self.repo_url)),
            escape(&format!(
                "{}#6-ui-parity-strategy",
                self.blob("docs/rust-rewrite/PLAN.md")
            ))
        );
        if !m.problems.is_empty() {
            w!(
                html,
                "<div class=\"reason\"><p><strong>{} problem(s) in the matrix</strong> (<code>cargo xtask \
                 parity-matrix</code>); the counts below may be off.</p>\n<ul>\n",
                m.problems.len()
            );
            for problem in &m.problems {
                w!(html, "<li><code>{}</code></li>\n", escape(&problem.to_string()));
            }
            html.push_str("</ul></div>\n");
        }

        let mut status_header = vec![""];
        status_header.extend(STATUSES);
        status_header.push("Total");

        html.push_str("<h3>Status by milestone</h3>\n");
        status_header[0] = "Milestone";
        let rows: Vec<(String, Vec<usize>)> = m
            .counts_by_milestone()
            .into_iter()
            .map(|(name, c)| (escape(name), with_total(&c)))
            .collect();
        html.push_str(&table(&status_header, &rows));

        html.push_str("<h3>Status by area</h3>\n");
        status_header[0] = "Area";
        let area_link = |name: &str| {
            format!(
                "<a href=\"{}\">{}</a>",
                escape(&self.blob(&format!("{MATRIX_DIR}/areas/{name}.md"))),
                escape(name)
            )
        };
        let rows: Vec<(String, Vec<usize>)> = m
            .counts()
            .into_iter()
            .map(|(name, c)| (area_link(name), with_total(&c)))
            .collect();
        html.push_str(&table(&status_header, &rows));

        html.push_str(
            "<h3>Coverage</h3>\n<p class=\"muted\">How many of each area's rows a parity scenario, a Python \
                       golden, both or neither names.</p>\n",
        );
        let mut coverage_header = vec!["Area", "Rows"];
        coverage_header.extend(COVERAGE);
        let rows: Vec<(String, Vec<usize>)> = m
            .coverage_counts()
            .into_iter()
            .map(|(name, c)| (area_link(name), c.to_vec()))
            .collect();
        html.push_str(&table(&coverage_header, &rows));
        html
    }
}

/// The counts followed by their sum.
fn with_total(counts: &[usize]) -> Vec<usize> {
    let mut row = counts.to_vec();
    row.push(counts.iter().sum());
    row
}

/// An HTML table of `rows` (an already-escaped label, then numbers) under `header`, with a Total row.
fn table(header: &[&str], rows: &[(String, Vec<usize>)]) -> String {
    let mut html = String::from("<table>\n<tr>");
    for (i, h) in header.iter().enumerate() {
        w!(
            html,
            "<th{}>{}</th>",
            if i == 0 { "" } else { " class=\"n\"" },
            escape(h)
        );
    }
    html.push_str("</tr>\n");
    let mut totals = vec![0; header.len().saturating_sub(1)];
    for (label, counts) in rows {
        w!(html, "<tr><td>{label}</td>");
        for (total, n) in totals.iter_mut().zip(counts) {
            *total += n;
            w!(html, "<td class=\"n\">{n}</td>");
        }
        html.push_str("</tr>\n");
    }
    html.push_str("<tr class=\"total\"><td>Total</td>");
    for n in totals {
        w!(html, "<td class=\"n\">{n}</td>");
    }
    html.push_str("</tr>\n</table>\n");
    html
}

/// Added to the report's stylesheet: numeric columns and the total row.
const SITE_CSS: &str = ".n{text-align:right}tr.total td{font-weight:600}.reason ul{margin:4px 0}\n";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::{Area, Covered, Problem, Row};
    use crate::parity::report::AppSource;
    use crate::parity::summary::ScenarioSummary;

    fn meta() -> RunMeta {
        RunMeta {
            git_sha: "0123456789abcdef0123456789abcdef01234567".into(),
            subject: "Add the <dialog> focus ring".into(),
            timestamp: "2026-10-26T12:00:00Z".into(),
            os: "linux (x86_64)".into(),
            command: "parity --all --no-sync --jobs 4".into(),
            python: AppSource::Recorded,
            rust: AppSource::Live,
        }
    }

    fn scenario(name: &str, result: ScenarioResult) -> ScenarioSummary {
        ScenarioSummary {
            name: name.into(),
            description: String::new(),
            gate: result != ScenarioResult::RustNotReady,
            parity_rows: Vec::new(),
            result,
            sizes: Vec::new(),
            error: None,
        }
    }

    fn row(id: &str, milestone: &str, status: &str) -> Row {
        Row {
            id: id.into(),
            line: 1,
            status: Some(status.into()),
            milestone: Some(milestone.into()),
            source: String::new(),
        }
    }

    fn matrix() -> Matrix {
        let mut m = Matrix {
            areas: vec![
                Area {
                    name: "main-layout".into(),
                    file: PathBuf::from("areas/main-layout.md"),
                    rows: vec![
                        row("main-layout.footer", "M0", "parity"),
                        row("main-layout.panes", "M1", "in progress"),
                        row("main-layout.theme", "M1", "intentional diff (ansi)"),
                    ],
                },
                Area {
                    name: "issues".into(),
                    file: PathBuf::from("areas/issues.md"),
                    rows: vec![row("issues.list", "M2", "not started")],
                },
            ],
            ..Matrix::default()
        };
        m.coverage.insert(
            "main-layout.footer".into(),
            Covered {
                scenarios: vec![PathBuf::from("parity/scenarios/startup.toml")],
                goldens: Vec::new(),
            },
        );
        m
    }

    fn site(repo_url: &str) -> Site {
        let mut failed = scenario("repo-select", ScenarioResult::Fail);
        failed.error = Some("fixture <miss>".into());
        Site {
            meta: meta(),
            summary: Summary {
                version: 1,
                scenarios: vec![
                    scenario("startup", ScenarioResult::Pass),
                    scenario("help", ScenarioResult::Pass),
                    failed,
                    scenario("pr-list", ScenarioResult::RustNotReady),
                ],
            },
            matrix: matrix(),
            repo_url: repo_url.into(),
        }
    }

    /// The `<h3>{title}</h3>` section's table, up to its end.
    fn section<'a>(html: &'a str, title: &str) -> &'a str {
        let start = html.find(&format!("<h3>{title}</h3>")).unwrap();
        let len = html[start..].find("</table>").unwrap();
        &html[start..start + len]
    }

    #[test]
    fn repo_url_comes_from_the_actions_environment() {
        assert_eq!(repo_url(None, None), DEFAULT_REPO_URL);
        assert_eq!(repo_url(Some("https://github.com"), Some("")), DEFAULT_REPO_URL);
        assert_eq!(
            repo_url(Some("https://github.com"), Some("someone/fork")),
            "https://github.com/someone/fork"
        );
        assert_eq!(
            repo_url(Some("https://ghe.example.com/"), Some("org/repo")),
            "https://ghe.example.com/org/repo"
        );
        assert_eq!(repo_url(None, Some("org/repo")), "https://github.com/org/repo");
    }

    #[test]
    fn page_links_the_commit_with_and_without_github_repository() {
        let html = site(&repo_url(None, None)).render();
        assert!(
            html.contains(
                "<a href=\"https://github.com/gizmo385/gh-lazy/commit/0123456789abcdef0123456789abcdef01234567\">\
                 <code>0123456</code></a> Add the &lt;dialog&gt; focus ring"
            ),
            "{html}"
        );
        assert!(html.contains("<td>2026-10-26T12:00:00Z</td>"));
        assert!(html.contains("href=\"https://github.com/gizmo385/gh-lazy/tree/rust-main/docs/rust-rewrite/parity\""));
        assert!(html.contains(
            "href=\"https://github.com/gizmo385/gh-lazy/blob/rust-main/docs/rust-rewrite/PLAN.md#6-ui-parity-strategy\""
        ));

        let fork = site(&repo_url(Some("https://github.com"), Some("someone/fork"))).render();
        assert!(fork.contains("<a href=\"https://github.com/someone/fork/commit/0123456789abcdef"));
        assert!(!fork.contains("gizmo385"));

        // A run outside git has no commit to link.
        let mut unknown = site(DEFAULT_REPO_URL);
        unknown.meta.git_sha = "unknown".into();
        unknown.meta.subject = String::new();
        assert!(
            unknown
                .render()
                .contains("<tr><th>Commit</th><td><code>unknown</code></td></tr>")
        );
    }

    #[test]
    fn page_shows_the_counts() {
        let html = site(DEFAULT_REPO_URL).render();
        assert!(
            html.contains(
                "<strong>2</strong> <span class=\"badge b-pass\">pass</span> \
                 <strong>1</strong> <span class=\"badge b-fail\">fail</span> \
                 <strong>1</strong> <span class=\"badge b-rust-not-ready\">rust-not-ready</span> of 4 scenario(s). \
                 Python: recorded</p>"
            ),
            "{html}"
        );
        assert!(
            !html.contains("badge b-python-only"),
            "zero limited results are left out"
        );
        assert!(html.contains("<a href=\"report/index.html\">Full report</a>"));
        assert!(
            html.contains("<li><a href=\"report/repo-select/report.html\">repo-select</a>: fixture &lt;miss&gt;</li>")
        );

        insta::assert_snapshot!(format!(
            "{}\n{}\n{}",
            section(&html, "Status by milestone"),
            section(&html, "Status by area"),
            section(&html, "Coverage")
        ));
    }

    #[test]
    fn page_lists_matrix_problems_instead_of_failing() {
        let mut s = site(DEFAULT_REPO_URL);
        s.matrix.problems.push(Problem {
            file: PathBuf::from("docs/rust-rewrite/parity/areas/issues.md"),
            line: 7,
            message: "status `done` isn't one of <these>".into(),
        });
        let html = s.render();
        assert!(html.contains("<strong>1 problem(s) in the matrix</strong>"), "{html}");
        assert!(html.contains(
            "<li><code>docs/rust-rewrite/parity/areas/issues.md:7: status `done` isn&#39;t one of &lt;these&gt;</code></li>"
        ));
        // The tables are still there.
        assert!(html.contains("<h3>Status by milestone</h3>"));
        assert!(!site(DEFAULT_REPO_URL).render().contains("problem(s) in the matrix"));
    }

    #[test]
    fn page_references_no_external_assets() {
        let html = site(DEFAULT_REPO_URL).render();
        for needle in ["<link", " src=", "@import", "url(", "<script"] {
            assert!(!html.contains(needle), "found {needle:?}");
        }
    }

    #[test]
    fn a_report_without_index_html_is_an_error() {
        let repo = tempfile::tempdir().unwrap();
        let report = repo.path().join("report");
        std::fs::create_dir_all(&report).unwrap();
        let out = repo.path().join("out");
        let err = write(repo.path(), &report, &out, DEFAULT_REPO_URL.into()).unwrap_err();
        assert!(matches!(&err, SiteError::MissingReport(p) if *p == report), "{err}");
        assert!(
            err.to_string()
                .ends_with("has no index.html; the parity run wrote no report")
        );
        assert!(!out.exists(), "nothing is written");
    }

    #[test]
    fn writes_the_page_from_a_report_on_disk_despite_a_broken_matrix() {
        // An empty repo: no matrix at all, which the check reports as problems.
        let repo = tempfile::tempdir().unwrap();
        let report = repo.path().join("report");
        std::fs::create_dir_all(&report).unwrap();
        let summary = Summary {
            version: 1,
            scenarios: vec![scenario("startup", ScenarioResult::Pass)],
        };
        report::write_reports(&report, &summary, &meta()).unwrap();
        std::fs::write(report.join("summary.json"), serde_json::to_string(&summary).unwrap()).unwrap();

        let out = repo.path().join("site");
        let (path, site) = write(repo.path(), &report, &out, DEFAULT_REPO_URL.into()).unwrap();
        assert_eq!(path, out.join("index.html"));
        assert!(!site.matrix.problems.is_empty());
        let html = std::fs::read_to_string(path).unwrap();
        assert!(html.contains("<code>0123456</code></a> Add the &lt;dialog&gt; focus ring"));
        assert!(html.contains("<strong>1</strong> <span class=\"badge b-pass\">pass</span>"));
        assert!(html.contains("problem(s) in the matrix"));

        // A report from before meta.json existed can't be published.
        std::fs::remove_file(report.join(report::META_FILE)).unwrap();
        let err = write(repo.path(), &report, &out, DEFAULT_REPO_URL.into()).unwrap_err();
        assert!(err.to_string().contains("meta.json"), "{err}");
    }
}
