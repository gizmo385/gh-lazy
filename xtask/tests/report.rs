//! Writes a report from synthetic dumps on disk: the text diff, `check.json` and `structure.json` when present,
//! a failed run's screen, and the index and `summary.md`. Needs no PTY and no Python.

use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::path::Path;

use xtask::parity::grid::Grid;
use xtask::parity::report::{AppSource, RunMeta, write_reports};
use xtask::parity::runner::{AppRunStatus, RunState};
use xtask::parity::scenario::App;
use xtask::parity::summary::{CaptureSummary, ScenarioResult, ScenarioSummary, SizeSummary, Summary};

const SIZE: &str = "24x4";

fn screen(bytes: &[u8]) -> Grid {
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

fn scenario(name: &str, result: ScenarioResult, rust: RunState) -> ScenarioSummary {
    ScenarioSummary {
        name: name.into(),
        description: format!("<{name}> & friends"),
        gate: false,
        parity_rows: vec!["main-layout.layout".into()],
        result,
        sizes: vec![SizeSummary {
            size: SIZE.into(),
            apps: BTreeMap::from([(App::Python, status(RunState::Ok)), (App::Rust, status(rust))]),
            captures: vec![CaptureSummary::unchecked("main")],
        }],
        error: None,
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn writes_every_report_file_from_the_dumps() {
    let out = tempfile::tempdir().unwrap();
    let out = out.path();

    // `demo`: both apps captured, one cell differs, #93's files are present.
    let demo = out.join("demo").join(SIZE);
    write_grid(
        &demo.join("python"),
        "main",
        &screen(b"\x1b[38;2;78;191;113m[1] Repositories\x1b[0m\r\nalpha <b>\r\nq Quit"),
    );
    write_grid(
        &demo.join("rust"),
        "main",
        &screen(b"\x1b[38;2;78;191;113m[1] Repositories\x1b[0m\r\nalphA <b>\r\nq Quit"),
    );
    write(
        &demo.join("python/main.structure.json"),
        r#"{"version":1,"focused":["[1] Repositories"]}"#,
    );
    write(
        &demo.join("python/fake-gh.log"),
        "{\"seq\":1,\"command\":\"auth status\"}\n",
    );
    write(
        &demo.join("main.check.json"),
        r#"{"capture":"main","size":[24,4],
            "expect":[{"check":"titles","box":null,"python":["[9] Repos"],"rust":null,"expected":["[1] Repos"],
                       "message":"pane titles differ"}],
            "cross":[{"check":"rows","box":"[1] Repositories","python":"alpha","rust":"alphA","message":"cell differs"}]}"#,
    );

    // `broken`: the Rust app exited before capturing; only its failure screen exists. No check.json.
    let broken = out.join("broken").join(SIZE);
    write_grid(&broken.join("python"), "main", &screen(b"hello"));
    write_grid(&broken.join("rust"), "failure", &screen(b"Error: <panic> & exit"));

    let summary = Summary {
        version: 1,
        scenarios: vec![
            scenario("demo", ScenarioResult::RustNotReady, RunState::Ok),
            scenario("broken", ScenarioResult::RustNotReady, RunState::Exited),
        ],
    };
    let meta = RunMeta {
        git_sha: "abc123".into(),
        timestamp: "2026-01-01T00:00:00Z".into(),
        os: "test".into(),
        command: "parity demo broken".into(),
        python: AppSource::Recorded,
        rust: AppSource::Live,
    };
    write_reports(out, &summary, &meta).unwrap();

    let diff = read(&demo.join("main.diff.txt"));
    assert!(diff.starts_with("--- python\n+++ rust\n"), "{diff}");
    assert!(diff.contains("-alpha <b>\n+alphA <b>\n"), "{diff}");
    assert!(!broken.join("main.diff.txt").exists(), "no diff without both captures");

    let report = read(&out.join("demo/report.html"));
    assert!(report.contains("&lt;demo&gt; &amp; friends"));
    assert!(report.contains("1 cell(s) differ"));
    assert!(report.contains(
        "<li><code>main-layout.layout</code> in <code>docs/rust-rewrite/parity/areas/main-layout.md</code></li>"
    ));
    assert!(report.contains("<td><code>alpha</code></td><td><code>alphA</code></td><td></td><td>cell differs</td>"));
    assert!(
        report.contains(
            "<td><code>(none)</code></td><td><code>[&quot;[1] Repos&quot;]</code></td><td>pane titles differ</td>"
        ),
        "an [expect] mismatch shows what was expected"
    );
    assert!(
        report.contains("&quot;focused&quot;: ["),
        "structure.json is pretty-printed"
    );
    assert!(report.contains("fake-gh.log (python, last 50 lines)"));
    assert!(report.contains("&lt;b&gt;"));

    let broken_report = read(&out.join("broken/report.html"));
    assert!(broken_report.contains("No capture: exited with code 1 at step 0 (0.9s)."));
    assert!(broken_report.contains("Error: &lt;panic&gt; &amp; exit"));
    assert!(broken_report.contains("not checked"));

    let index = read(&out.join("index.html"));
    assert!(index.contains("<a href=\"demo/report.html\">demo</a>"));
    assert!(index.contains("<code>cargo xtask parity demo broken</code>"));

    let md = read(&out.join("summary.md"));
    assert!(md.starts_with("## Parity: 0 pass, 0 fail, 2 rust-not-ready\n"), "{md}");
    assert!(
        md.contains("| demo | no | skipped | skipped | rust-not-ready | 2 |"),
        "{md}"
    );
    assert!(
        md.contains("- demo/24x4/main: rows [1] Repositories: python=alpha rust=alphA"),
        "{md}"
    );
    assert!(
        md.contains(r#"- demo/24x4/main: titles: python=["[9] Repos"] rust=(none) expected=["[1] Repos"]"#),
        "{md}"
    );
}
