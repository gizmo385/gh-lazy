//! Writes a report from synthetic dumps on disk: `check.json` and `structure.json` when present, a run that differs
//! from its snapshot, a failed run's screen, and the index and `summary.md`. Needs no PTY.

use std::num::NonZeroU16;
use std::path::Path;

use xtask::parity::check::{self, CheckFile, Kind};
use xtask::parity::ci;
use xtask::parity::grid::Grid;
use xtask::parity::report::{META_FILE, RunMeta, write_reports};
use xtask::parity::runner::{AppRunStatus, RunState};
use xtask::parity::scenario::{Roots, Scenario};
use xtask::parity::screen;
use xtask::parity::structure::ThemeColors;
use xtask::parity::summary::{CaptureSummary, CheckOutcome, ScenarioResult, ScenarioSummary, SizeSummary, Summary};

const SIZE: &str = "24x4";

fn grid(bytes: &[u8]) -> Grid {
    let mut parser = vt100::Parser::new(NonZeroU16::new(4).unwrap(), NonZeroU16::new(24).unwrap(), 0);
    parser.process(bytes);
    Grid::from_screen(parser.screen())
}

fn status(state: RunState) -> AppRunStatus {
    serde_json::from_value(serde_json::json!({
        "status": state, "step": if state == RunState::Ok { None } else { Some(0) },
        "exit_code": if state == RunState::Exited { Some(1) } else { None }, "duration_ms": 900, "flaky": false,
        "fixture_problems": false, "misses": [], "errors": []
    }))
    .unwrap()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn write_grid(dir: &Path, name: &str, grid: &Grid) {
    write(&dir.join(format!("{name}.grid.json")), &grid.to_json());
    write(&dir.join(format!("{name}.txt")), &grid.to_text());
}

fn scenario(name: &str, result: ScenarioResult, state: RunState, mismatches: usize) -> ScenarioSummary {
    ScenarioSummary {
        name: name.into(),
        description: format!("<{name}> & friends"),
        parity_rows: vec!["main-layout.layout".into()],
        result,
        sizes: vec![SizeSummary {
            size: SIZE.into(),
            status: status(state),
            captures: vec![CaptureSummary {
                name: "main".into(),
                check: if mismatches == 0 {
                    CheckOutcome::NotApplicable
                } else {
                    CheckOutcome::Fail
                },
                mismatches,
            }],
        }],
        error: None,
    }
}

fn meta(command: &str) -> RunMeta {
    RunMeta {
        git_sha: "abc123".into(),
        subject: "Add <things>".into(),
        timestamp: "2026-01-01T00:00:00Z".into(),
        os: "test".into(),
        command: command.into(),
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn writes_every_report_file_from_the_dumps() {
    let out = tempfile::tempdir().unwrap();
    let out = out.path();

    // `demo`: one cell differs from the snapshot, and an `[expect]` failed. Its structure and log are present.
    let demo = out.join("demo").join(SIZE);
    let snapshot = grid(b"\x1b[38;2;78;191;113m[1] Repositories\x1b[0m\r\nalpha <b>\r\nq Quit");
    let run = grid(b"\x1b[38;2;78;191;113m[1] Repositories\x1b[0m\r\nalphA <b>\r\nq Quit");
    write_grid(&demo, "main", &run);
    write(&demo.join("main.screen.snapshot"), &screen::render(&snapshot).unwrap());
    write(&demo.join("main.screen.new"), &screen::render(&run).unwrap());
    write(
        &demo.join("main.snapshot.diff.txt"),
        "--- snapshot\n+++ run\n-alpha <b>\n+alphA <b>\n",
    );
    write(
        &demo.join("main.structure.json"),
        r#"{"version":1,"focused":["[1] Repositories"]}"#,
    );
    write(&demo.join("fake-gh.log"), "{\"seq\":1,\"command\":\"auth status\"}\n");
    write(
        &demo.join("main.check.json"),
        r#"{"capture":"main","size":[24,4],
            "expect":[{"check":"titles","box":null,"actual":null,"expected":["[1] Repos"],
                       "message":"pane titles differ"}],
            "snapshot":[{"check":"snapshot","box":null,"actual":null,"message":"1 line(s) differ"}]}"#,
    );

    // `broken`: the app exited before capturing; only its failure screen exists. No check.json.
    let broken = out.join("broken").join(SIZE);
    write_grid(&broken, "failure", &grid(b"Error: <panic> & exit"));

    let summary = Summary {
        version: 2,
        scenarios: vec![
            scenario("demo", ScenarioResult::Fail, RunState::Ok, 2),
            scenario("broken", ScenarioResult::Fail, RunState::Exited, 0),
        ],
    };
    let meta = meta("parity demo broken");
    write_reports(out, &summary, &meta).unwrap();

    let report = read(&out.join("demo/report.html"));
    assert!(report.contains("&lt;demo&gt; &amp; friends"));
    assert!(report.contains("1 cell(s) differ"), "{report}");
    assert!(report.contains("<figcaption>Snapshot</figcaption>"));
    assert!(report.contains("<figcaption>Run</figcaption>"));
    assert!(report.contains(
        "<li><code>main-layout.layout</code> in <code>docs/rust-rewrite/parity/areas/main-layout.md</code></li>"
    ));
    assert!(
        report.contains(
            "<td><code>(none)</code></td><td><code>[&quot;[1] Repos&quot;]</code></td><td>pane titles differ</td>"
        ),
        "an [expect] mismatch shows what was expected: {report}"
    );
    assert!(
        report.contains("<td>1 line(s) differ <a href=\"24x4/main.snapshot.diff.txt\">diff</a></td>"),
        "{report}"
    );
    assert!(report.contains("+alphA &lt;b&gt;"), "the text diff is shown");
    assert!(
        report.contains("&quot;focused&quot;: ["),
        "structure.json is pretty-printed"
    );
    assert!(report.contains("fake-gh.log (last 50 lines)"));

    let broken_report = read(&out.join("broken/report.html"));
    assert!(broken_report.contains("No capture: exited with code 1 at step 0 (0.9s)."));
    assert!(broken_report.contains("Error: &lt;panic&gt; &amp; exit"));
    assert!(broken_report.contains("not checked"));

    let index = read(&out.join("index.html"));
    assert!(index.contains("<a href=\"demo/report.html\">demo</a>"));
    assert!(index.contains("<code>cargo xtask parity demo broken</code>"));
    assert!(index.contains("<p>0 pass, 2 fail</p>"), "{index}");

    let md = read(&out.join("summary.md"));
    assert!(md.starts_with("## Scenarios: 0 pass, 2 fail\n"), "{md}");
    assert!(md.contains("| demo | skipped | skipped | fail | 2 |"), "{md}");
    assert!(
        md.contains(r#"- demo/24x4/main: titles: actual=(none) expected=["[1] Repos"]"#),
        "{md}"
    );
    assert!(md.contains("- demo/24x4/main: snapshot: 1 line(s) differ"), "{md}");

    // The run metadata sits next to summary.md, machine-readable, and reads back as written.
    assert_eq!(RunMeta::read(out).unwrap(), meta);
    let json: serde_json::Value = serde_json::from_str(&read(&out.join(META_FILE))).unwrap();
    assert_eq!(json["git_sha"], "abc123");
    assert!(json.get("python").is_none(), "{json}");
    assert!(RunMeta::read(&out.join("missing")).is_err());
}

/// Checks scenario `titled` in `out`: two captures of the same screen, the last one expecting a title it doesn't
/// show.
fn check_titled(out: &Path) -> (Vec<CaptureSummary>, Vec<ci::Finding>) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let roots = Roots {
        fixtures: repo.join("parity/fixtures"),
        configs: repo.join("parity/configs"),
        seeds: repo.join("parity/seeds"),
    };
    let source = "version = 2\n[[step]]\ncapture = \"first\"\n[[step]]\ncapture = \"last\"\n\
                  [expect.last]\npane_titles = [\"[9] Repos\"]\n";
    let scenario = Scenario::parse(Path::new("titled.toml"), source, &roots).unwrap();

    let dir = out.join("titled").join(SIZE);
    let boxed = grid("┌─ [1] Repos ─┐\r\n│ alpha       │\r\n└─────────────┘".as_bytes());
    for capture in ["first", "last"] {
        write_grid(&dir, capture, &boxed);
    }
    check::check_size(&scenario, &dir, &ThemeColors::default(), None).unwrap()
}

#[test]
fn an_expect_mismatch_reaches_check_json_ci_and_the_report() {
    let out = tempfile::tempdir().unwrap();
    let out = out.path();
    let dir = out.join("titled").join(SIZE);
    let (captures, findings) = check_titled(out);

    // The first capture has no `[expect]` and no snapshot was compared: nothing checked it.
    assert_eq!(captures[0].check, CheckOutcome::NotApplicable);
    assert!(!dir.join("first.check.json").exists());
    assert!(dir.join("first.structure.json").exists());
    assert_eq!((captures[1].check, captures[1].mismatches), (CheckOutcome::Fail, 1));
    let check: CheckFile = serde_json::from_str(&read(&dir.join("last.check.json"))).unwrap();
    assert_eq!(check.expect.len(), 1, "{check:#?}");
    assert!(check.snapshot.is_empty());
    let m = &check.expect[0];
    assert_eq!(m.check, Kind::Titles);
    assert_eq!(m.actual, serde_json::json!(["[1] Repos"]));
    assert_eq!(m.expected, Some(serde_json::json!(["[9] Repos"])));

    let sizes = vec![SizeSummary {
        size: SIZE.into(),
        status: status(RunState::Ok),
        captures,
    }];
    let summary = ScenarioSummary {
        name: "titled".into(),
        description: String::new(),
        parity_rows: Vec::new(),
        result: ScenarioResult::from_sizes(&sizes),
        sizes,
        error: None,
    };
    assert_eq!(summary.result, ScenarioResult::Fail);
    let annotations: Vec<String> = ci::annotations("parity/scenarios/titled.toml", &summary, &findings)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        annotations
            .iter()
            .any(|a| a.starts_with("::error file=parity/scenarios/titled.toml::")
                && a.contains("titled/24x4/last: expect titles")),
        "{annotations:#?}"
    );

    let summary = Summary {
        version: 2,
        scenarios: vec![summary],
    };
    write_reports(out, &summary, &meta("parity titled")).unwrap();
    let md = read(&out.join("summary.md"));
    assert!(md.contains("| titled | skipped | skipped | fail | 1 |"), "{md}");
    assert!(
        md.contains(r#"- titled/24x4/last: titles: actual=["[1] Repos"] expected=["[9] Repos"]"#),
        "{md}"
    );
    let report = read(&out.join("titled/report.html"));
    assert!(
        report.contains(
            "<tr><td>titles <span class=\"muted\">(expect)</span></td><td></td>\
             <td><code>[&quot;[1] Repos&quot;]</code></td><td><code>[&quot;[9] Repos&quot;]</code></td>"
        ),
        "{report}"
    );
    assert!(
        report.contains("<p class=\"muted\">not checked</p>"),
        "the first capture"
    );
}
