//! Writes a report from synthetic dumps on disk: the text diff, `check.json` and `structure.json` when present,
//! a failed run's screen, and the index and `summary.md`. Needs no PTY and no Python.

use std::collections::BTreeMap;
use std::num::NonZeroU16;
use std::path::Path;

use xtask::parity::check::{self, CheckFile, Kind};
use xtask::parity::ci;
use xtask::parity::grid::Grid;
use xtask::parity::report::{AppSource, META_FILE, RunMeta, write_reports};
use xtask::parity::runner::{AppRunStatus, RunState};
use xtask::parity::scenario::{App, Roots, Scenario};
use xtask::parity::structure::ThemeColors;
use xtask::parity::summary::{CaptureSummary, CheckOutcome, ScenarioResult, ScenarioSummary, SizeSummary, Summary};

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
        subject: "Add <things>".into(),
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

    // The run metadata sits next to summary.md, machine-readable, and reads back as written.
    assert_eq!(RunMeta::read(out).unwrap(), meta);
    let json: serde_json::Value = serde_json::from_str(&read(&out.join(META_FILE))).unwrap();
    assert_eq!(json["git_sha"], "abc123");
    assert_eq!(json["python"], "recorded");
}

/// A fake-gh log line for a served `method path` request with `body`.
fn log_line(method: &str, path: &str, body: &serde_json::Value) -> String {
    let entry = serde_json::json!({
        "seq": 1, "argv": ["api", path], "command": "api", "method": method, "path": path, "query": {},
        "accept": "json", "cache": null, "cache_control": null, "body": body, "fixture": "base/x.json",
        "status": 200, "miss": false, "error": null
    });
    format!("{entry}\n")
}

/// Runs the structural check on scenario `writer` in `out`: two captures with the same screen in both apps, and
/// request logs that agree on a `PUT` but each have a `POST` the other doesn't (Rust's sent twice).
fn check_writer(out: &Path) -> (BTreeMap<App, AppRunStatus>, Vec<CaptureSummary>, Vec<ci::Finding>) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let roots = Roots {
        fixtures: repo.join("parity/fixtures"),
        configs: repo.join("parity/configs"),
        seeds: repo.join("parity/seeds"),
    };
    let source = "version = 1\ngate = false\n[[step]]\ncapture = \"first\"\n[[step]]\ncapture = \"last\"\n";
    let scenario = Scenario::parse(Path::new("writer.toml"), source, &roots).unwrap();

    let dir = out.join("writer").join(SIZE);
    let get = log_line("GET", "/user", &serde_json::Value::Null);
    let shared = log_line(
        "PUT",
        "/repos/o/r/pulls/1/merge",
        &serde_json::json!({"merge_method": "squash"}),
    );
    let python_only = log_line(
        "POST",
        "/repos/o/r/issues",
        &serde_json::json!({"title": "x", "body": "y"}),
    );
    let rust_only = log_line("POST", "/repos/o/r/issues", &serde_json::json!({"title": "x"}));
    for (app, log) in [
        (App::Python, [get.as_str(), &shared, &python_only].concat()),
        (App::Rust, [shared.as_str(), &rust_only, &rust_only, &get].concat()),
    ] {
        let app_dir = dir.join(app.name());
        for capture in ["first", "last"] {
            write_grid(&app_dir, capture, &screen(b"same screen"));
        }
        write(&app_dir.join("fake-gh.log"), &log);
    }
    let statuses = BTreeMap::from([(App::Python, status(RunState::Ok)), (App::Rust, status(RunState::Ok))]);
    let (captures, findings) = check::check_size(&scenario, &dir, &statuses, &ThemeColors::default(), &[]).unwrap();
    (statuses, captures, findings)
}

#[test]
fn a_write_sent_by_only_one_app_shows_on_the_last_capture() {
    let out = tempfile::tempdir().unwrap();
    let out = out.path();
    let dir = out.join("writer").join(SIZE);
    let (statuses, captures, findings) = check_writer(out);

    // Only the last capture carries it, against Rust.
    assert_eq!((captures[0].check, captures[0].mismatches), (CheckOutcome::Pass, 0));
    assert_eq!((captures[1].check, captures[1].mismatches), (CheckOutcome::Fail, 1));
    assert_eq!(captures[1].against, [App::Rust]);
    let check: CheckFile = serde_json::from_str(&read(&dir.join("last.check.json"))).unwrap();
    assert_eq!(check.cross.len(), 1, "{check:#?}");
    let m = &check.cross[0];
    assert_eq!(m.check, Kind::Writes);
    assert_eq!(
        m.message,
        r#"2 writes differ: only in python: POST /repos/o/r/issues {"body":"y","title":"x"}; only in rust: POST /repos/o/r/issues {"title":"x"}"#
    );
    assert_eq!(
        m.python,
        serde_json::json!([r#"POST /repos/o/r/issues {"body":"y","title":"x"}"#])
    );
    assert_eq!(m.rust, serde_json::json!([r#"POST /repos/o/r/issues {"title":"x"}"#]));

    let sizes = vec![SizeSummary {
        size: SIZE.into(),
        apps: statuses,
        captures,
    }];
    let summary = ScenarioSummary {
        name: "writer".into(),
        description: String::new(),
        gate: false,
        parity_rows: Vec::new(),
        result: ScenarioResult::from_statuses(false, &sizes),
        sizes,
        error: None,
    };
    assert_eq!(summary.result, ScenarioResult::RustNotReady);
    // Gated, it fails the scenario, and `--ci` names the write.
    let gated = ScenarioSummary {
        gate: true,
        result: ScenarioResult::from_statuses(true, &summary.sizes),
        ..summary.clone()
    };
    assert_eq!(gated.result, ScenarioResult::Fail);
    let annotations: Vec<String> = ci::annotations("parity/scenarios/writer.toml", &gated, &findings)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        annotations
            .iter()
            .any(|a| a.contains("writer/24x4/last: cross writes: 2 writes differ: only in python: POST")),
        "{annotations:#?}"
    );

    let meta = RunMeta {
        git_sha: "abc123".into(),
        subject: String::new(),
        timestamp: "2026-01-01T00:00:00Z".into(),
        os: "test".into(),
        command: "parity writer".into(),
        python: AppSource::Recorded,
        rust: AppSource::Live,
    };
    let summary = Summary {
        version: 1,
        scenarios: vec![summary],
    };
    write_reports(out, &summary, &meta).unwrap();
    let md = read(&out.join("summary.md"));
    assert!(
        md.contains("| writer | no | skipped | skipped | rust-not-ready | 1 |"),
        "{md}"
    );
    assert!(
        md.contains(
            r#"- writer/24x4/last: writes: python=["POST /repos/o/r/issues {\"body\":\"y\",\"title\":\"x\"}"]"#
        ),
        "{md}"
    );
    let report = read(&out.join("writer/report.html"));
    assert!(
        report.contains(
            "<tr><td>writes <span class=\"muted\">(cross)</span></td><td></td><td><code>[&quot;POST \
             /repos/o/r/issues"
        ),
        "{report}"
    );
    assert!(
        report.contains(
            "<td>2 writes differ: only in python: POST /repos/o/r/issues \
             {&quot;body&quot;:&quot;y&quot;,&quot;title&quot;:&quot;x&quot;}; only in rust:"
        ),
        "{report}"
    );
}

#[test]
fn run_metadata_round_trips_every_app_source() {
    let out = tempfile::tempdir().unwrap();
    let meta = RunMeta {
        git_sha: "abc123".into(),
        subject: String::new(),
        timestamp: "2026-01-01T00:00:00Z".into(),
        os: "test".into(),
        command: "parity --all".into(),
        python: AppSource::LiveFor(vec!["a".into(), "b".into()]),
        rust: AppSource::NotRun,
    };
    let summary = Summary {
        version: 1,
        scenarios: Vec::new(),
    };
    write_reports(out.path(), &summary, &meta).unwrap();
    assert_eq!(RunMeta::read(out.path()).unwrap(), meta);
    assert!(RunMeta::read(&out.path().join("missing")).is_err());
}
