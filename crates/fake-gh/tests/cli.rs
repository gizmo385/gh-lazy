//! End-to-end tests of the fake-gh binary, run as `gh` the way the harness installs it.

mod common;

use std::io::Write;
use std::process::{Output, Stdio};

use common::{TempDir, gh, install_gh, read_log, write};
use serde_json::{Value, json};

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

fn page_fixture(page: u32, next: Option<u32>, body: &Value) -> String {
    let headers = next.map_or_else(
        || json!({}),
        |n| json!({"Link": format!("<https://api.github.com/repos/o/r/issues?page={n}&per_page=2>; rel=\"next\"")}),
    );
    json!({
        "version": 1,
        "request": {"method": "GET", "path": "/repos/o/r/issues", "query": {"page": page.to_string(), "per_page": "2"}},
        "response": {"status": 200, "headers": headers, "body": body}
    })
    .to_string()
}

struct Setup {
    tmp: TempDir,
    bin: std::path::PathBuf,
    set: std::path::PathBuf,
    log: std::path::PathBuf,
}

fn setup(tag: &str) -> Setup {
    let tmp = TempDir::new(tag);
    let bin = install_gh(tmp.path());
    let set = tmp.path().join("fixtures").join("base");
    std::fs::create_dir_all(&set).unwrap();
    let log = tmp.path().join("fake-gh.log");
    Setup { tmp, bin, set, log }
}

impl Setup {
    fn gh(&self) -> std::process::Command {
        gh(&self.bin, &[&self.set], Some(&self.log))
    }

    fn run(&self, args: &[&str]) -> Output {
        self.gh().args(args).output().unwrap()
    }
}

#[test]
fn paginate_prints_pages_back_to_back_and_slurp_wraps_them() {
    let s = setup("paginate");
    write(
        &s.set,
        "repos/o/r/issues/GET.page-1_per_page-2.json",
        &page_fixture(1, Some(2), &json!([{"n": 1}, {"n": 2}])),
    );
    write(
        &s.set,
        "repos/o/r/issues/GET.page-2_per_page-2.json",
        &page_fixture(2, None, &json!([{"n": 3}])),
    );

    // Like gh 2.89's paginatedArrayReader: array pages are merged into one array when there's no -i or --slurp.
    let plain = s.run(&["api", "--paginate", "/repos/o/r/issues?page=1&per_page=2"]);
    assert_eq!(plain.status.code(), Some(0), "{}", stderr(&plain));
    assert_eq!(stdout(&plain), r#"[{"n":1},{"n":2},{"n":3}]"#);

    let slurp = s.run(&[
        "api",
        "--paginate",
        "--slurp",
        "-X",
        "GET",
        "-F",
        "page=1",
        "-F",
        "per_page=2",
        "/repos/o/r/issues",
    ]);
    assert_eq!(slurp.status.code(), Some(0), "{}", stderr(&slurp));
    assert_eq!(stdout(&slurp), r#"[[{"n":1},{"n":2}],[{"n":3}]]"#);

    // With -i every page gets its own status line and headers.
    let include = s.run(&["api", "-i", "--paginate", "/repos/o/r/issues?page=1&per_page=2"]);
    let out = stdout(&include);
    assert_eq!(out.matches("HTTP/2.0 200 OK\n").count(), 2, "{out}");
    assert!(out.ends_with("\r\n\r\n[{\"n\":3}]"), "{out}");

    // Without --paginate, only the first page.
    assert_eq!(
        stdout(&s.run(&["api", "/repos/o/r/issues?page=1&per_page=2"])),
        r#"[{"n":1},{"n":2}]"#
    );
}

#[test]
fn paginate_without_page_starts_at_page_one() {
    let s = setup("paginate-no-page");
    for (page, next, body) in [
        (1, Some(2), json!([1, 2])),
        (2, Some(3), json!([3, 4])),
        (3, None, json!([5])),
    ] {
        write(
            &s.set,
            &format!("repos/o/r/issues/GET.page-{page}_per_page-2.json"),
            &page_fixture(page, next, &body),
        );
    }
    // Real gh's --paginate sends the first URL as given, with no page.
    let out = s.run(&["api", "--paginate", "/repos/o/r/issues?per_page=2"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "[1,2,3,4,5]");
    let log = read_log(&s.log);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["query"], json!({"page": "1", "per_page": "2"}));
    assert_eq!(log[0]["fixture"], "base/repos/o/r/issues/GET.page-1_per_page-2.json");
    assert_eq!(
        log[0]["argv"],
        json!(["api", "--paginate", "/repos/o/r/issues?per_page=2"])
    );
}

#[test]
fn paginate_prints_non_array_pages_back_to_back() {
    let s = setup("paginate-objects");
    let page = |page: u32, next: Option<u32>, body: &Value| {
        let headers = next.map_or_else(
            || json!({}),
            |n| json!({"Link": format!("<https://api.github.com/x?page={n}>; rel=\"next\"")}),
        );
        json!({"version": 1, "request": {"method": "GET", "path": "/x", "query": {"page": page.to_string()}},
               "response": {"status": 200, "headers": headers, "body": body}})
        .to_string()
    };
    write(
        &s.set,
        "x/GET.page-1.json",
        &page(1, Some(2), &json!({"total": 3, "items": [1]})),
    );
    write(
        &s.set,
        "x/GET.page-2.json",
        &page(2, Some(3), &json!({"total": 3, "items": [2]})),
    );
    write(&s.set, "x/GET.page-3.json", &page(3, None, &json!([])));
    let out = s.run(&["api", "--paginate", "/x?page=1"]);
    assert_eq!(stdout(&out), r#"{"total":3,"items":[1]}{"total":3,"items":[2]}[]"#);
}

#[test]
fn paginate_merges_empty_array_pages() {
    let s = setup("paginate-empty");
    write(
        &s.set,
        "repos/o/r/issues/GET.page-1_per_page-2.json",
        &page_fixture(1, Some(2), &json!([1, 2])),
    );
    write(
        &s.set,
        "repos/o/r/issues/GET.page-2_per_page-2.json",
        &page_fixture(2, None, &json!([])),
    );
    let out = s.run(&["api", "--paginate", "/repos/o/r/issues?page=1&per_page=2"]);
    assert_eq!(stdout(&out), "[1,2]");
}

#[test]
fn unwritable_log_is_reported_once() {
    let s = setup("bad-log");
    let missing = s.tmp.path().join("missing-dir").join("fake-gh.log");
    let out = gh(&s.bin, &[&s.set], Some(&missing))
        .args(["api", "/user"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let err = stderr(&out);
    assert_eq!(err.matches("can't open log").count(), 1, "{err}");
}

#[test]
fn paginate_with_a_missing_next_page_is_a_miss() {
    let s = setup("paginate-miss");
    write(
        &s.set,
        "repos/o/r/issues/GET.page-1_per_page-2.json",
        &page_fixture(1, Some(2), &json!([1, 2])),
    );
    let out = s.run(&["api", "--paginate", "/repos/o/r/issues?page=1&per_page=2"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out).starts_with("[1,2]{\"message\":\"Not Found\""),
        "{}",
        stdout(&out)
    );
    let log = read_log(&s.log);
    assert_eq!(log[0]["miss"], true);
    assert_eq!(
        log[0]["query"],
        json!({"page": "2", "per_page": "2"}),
        "the missing page's key is logged"
    );
}

#[test]
fn miss_prints_the_404_shape_and_is_logged() {
    let s = setup("miss");
    let out = s.run(&[
        "api",
        "-i",
        "-X",
        "GET",
        "-F",
        "page=1",
        "https://api.github.com/repos/o/r/nope/",
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout(&out),
        "HTTP/2.0 404 Not Found\nContent-Type: application/json; charset=utf-8\r\n\r\n\
         {\"message\":\"Not Found\",\"documentation_url\":\"https://docs.github.com/rest\",\"status\":\"404\"}"
    );
    assert_eq!(stderr(&out), "gh: Not Found (HTTP 404)\n");
    let log = read_log(&s.log);
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["miss"], true);
    assert_eq!(log[0]["path"], "/repos/o/r/nope");
    assert_eq!(log[0]["query"], json!({"page": "1"}));
    assert_eq!(log[0]["status"], 404);
    assert_eq!(log[0]["fixture"], Value::Null);
}

#[test]
fn foreign_hosts_are_misses() {
    let s = setup("foreign");
    let out = s.run(&["api", "https://example.com/user"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(read_log(&s.log)[0]["miss"], true);
}

#[test]
fn unsupported_flags_and_commands_exit_2() {
    let s = setup("unsupported");
    let out = s.run(&["api", "--jq", ".[].name", "/user/repos"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), "fake-gh: unsupported flag --jq\n");
    let out = s.run(&["repo", "view"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), "fake-gh: unsupported command: gh repo view\n");
    let out = s.run(&["api", "/repos/{owner}/{repo}/pulls"]);
    assert_eq!(out.status.code(), Some(2));
    // `lint` is only available when invoked as fake-gh.
    assert_eq!(s.run(&["lint", "x"]).status.code(), Some(2));

    let log = read_log(&s.log);
    assert_eq!(log.len(), 4);
    assert_eq!(log[0]["error"], "unsupported flag --jq");
    assert_eq!(log[0]["argv"], json!(["api", "--jq", ".[].name", "/user/repos"]));
    assert!(log.iter().all(|l| l["error"].is_string()));
}

#[test]
fn http_errors_go_to_stderr_with_exit_1() {
    let s = setup("http-error");
    write(
        &s.set,
        "repos/o/r/pulls/1/merge/PUT.json",
        &json!({"version": 1, "request": {"method": "PUT", "path": "/repos/o/r/pulls/1/merge"},
                "response": {"status": 405, "body": {"message": "Pull Request is not mergeable"}}})
        .to_string(),
    );
    let out = s.run(&["api", "-X", "PUT", "/repos/o/r/pulls/1/merge"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), r#"{"message":"Pull Request is not mergeable"}"#);
    assert_eq!(stderr(&out), "gh: Pull Request is not mergeable (HTTP 405)\n");
}

#[test]
fn body_and_when_matchers_with_input_from_stdin() {
    let s = setup("when");
    let merge = |extra: Value, status: u16| {
        let mut request = json!({"method": "PUT", "path": "/repos/o/r/pulls/1/merge"});
        request
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"version": 1, "request": request, "response": {"status": status, "body": {"merged": true}}}).to_string()
    };
    write(
        &s.set,
        "repos/o/r/pulls/1/merge/PUT.json",
        &merge(json!({"body": {"merge_method": "squash"}}), 200),
    );
    let pr = |state: &str, when: Option<Value>| {
        let mut request = json!({"method": "GET", "path": "/repos/o/r/pulls/1"});
        if let Some(w) = when {
            request["when"] = w;
        }
        json!({"version": 1, "request": request, "response": {"status": 200, "body": {"state": state}}}).to_string()
    };
    write(&s.set, "repos/o/r/pulls/1/GET.json", &pr("open", None));
    write(
        &s.set,
        "repos/o/r/pulls/1/GET.2.json",
        &pr(
            "closed",
            Some(json!({"after": [{"method": "PUT", "path": "/repos/o/r/pulls/1/merge"}]})),
        ),
    );

    assert_eq!(stdout(&s.run(&["api", "/repos/o/r/pulls/1"])), r#"{"state":"open"}"#);

    // The wrong body misses; the right one, via --input -, is served.
    let wrong = s.run(&[
        "api",
        "-X",
        "PUT",
        "-f",
        "merge_method=rebase",
        "/repos/o/r/pulls/1/merge",
    ]);
    assert_eq!(wrong.status.code(), Some(1));
    let mut child = s
        .gh()
        .args(["api", "-i", "-X", "PUT", "--input", "-", "/repos/o/r/pulls/1/merge"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"merge_method": "squash"}"#)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));

    assert_eq!(stdout(&s.run(&["api", "/repos/o/r/pulls/1"])), r#"{"state":"closed"}"#);

    let log = read_log(&s.log);
    assert_eq!(log[2]["body"], json!({"merge_method": "squash"}));
    assert_eq!(log[2]["fixture"], "base/repos/o/r/pulls/1/merge/PUT.json");
    assert_eq!(log[3]["fixture"], "base/repos/o/r/pulls/1/GET.2.json");
}

#[test]
fn pr_diff_prints_the_body_file() {
    let s = setup("diff");
    let diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";
    write(&s.set, "repos/o/r/pulls/12/GET.diff.body.diff", diff);
    write(
        &s.set,
        "repos/o/r/pulls/12/GET.diff.json",
        &json!({"version": 1, "request": {"method": "GET", "path": "/repos/o/r/pulls/12", "accept": "diff"},
                "response": {"status": 200, "headers": {"Content-Type": "text/x-diff"}, "body_file": "GET.diff.body.diff"}})
        .to_string(),
    );
    let out = s.run(&["pr", "diff", "-R", "o/r", "12", "--color", "never"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), diff);

    // The same fixture through `gh api` with the diff media type.
    let out = s.run(&[
        "api",
        "-H",
        "Accept: application/vnd.github.v3.diff",
        "/repos/o/r/pulls/12",
    ]);
    assert_eq!(stdout(&out), diff);

    let missing = s.run(&["pr", "diff", "-R", "o/r", "13"]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(stderr(&missing), "could not find pull request diff: HTTP 404\n");
    let log = read_log(&s.log);
    assert_eq!(log[0]["command"], "pr diff");
    assert_eq!(log[0]["accept"], "diff");
    assert_eq!(log[2]["miss"], true);
}

#[test]
fn auth_and_version() {
    let s = setup("auth");
    let out = s
        .gh()
        .args(["auth", "status"])
        .env("FAKE_GH_USER", "octo")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("Logged in to github.com account octo (keyring)"));
    let out = s
        .gh()
        .args(["auth", "status"])
        .env("FAKE_GH_AUTH", "logged-out")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "You are not logged into any GitHub hosts. To log in, run: gh auth login\n"
    );
    assert_eq!(stdout(&s.run(&["auth", "token"])), "fake-gh-token\n");
    let out = s
        .gh()
        .args(["auth", "token"])
        .env("FAKE_GH_AUTH", "logged-out")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let version = "gh version 2.89.0 (fake-gh)\nhttps://github.com/cli/cli/releases/tag/v2.89.0\n";
    assert_eq!(stdout(&s.run(&["--version"])), version);
    assert_eq!(stdout(&s.run(&["version"])), version);
    // Version works without any fake-gh configuration, so tools can detect fake-gh.
    let bare = gh(&s.bin, &[], None).arg("--version").output().unwrap();
    assert_eq!(stdout(&bare), version);

    let commands: Vec<Value> = read_log(&s.log).iter().map(|l| l["command"].clone()).collect();
    assert_eq!(
        commands,
        [
            "auth status",
            "auth status",
            "auth token",
            "auth token",
            "version",
            "version"
        ]
    );
}

#[test]
fn config_errors_exit_3() {
    let s = setup("config");
    let unset = gh(&s.bin, &[], None).args(["api", "/user"]).output().unwrap();
    assert_eq!(unset.status.code(), Some(3));
    assert_eq!(stderr(&unset), "fake-gh: FAKE_GH_FIXTURES is not set\n");

    let user = json!({"version": 1, "request": {"method": "GET", "path": "/user"}, "response": {"status": 200}});
    write(&s.set, "user/GET.json", &user.to_string());
    write(&s.set, "user/GET.2.json", &user.to_string());
    let dup = s.run(&["api", "/user"]);
    assert_eq!(dup.status.code(), Some(3));
    let err = stderr(&dup);
    assert!(
        err.contains("base/user/GET.2.json") && err.contains("base/user/GET.json"),
        "{err}"
    );
}

#[test]
fn earlier_set_overrides_later_end_to_end() {
    let s = setup("overlay");
    let overlay = s.tmp.path().join("fixtures").join("overlay");
    let user = |login: &str| {
        json!({"version": 1, "request": {"method": "GET", "path": "/user"}, "response": {"status": 200, "body": {"login": login}}})
            .to_string()
    };
    write(&s.set, "user/GET.json", &user("base"));
    write(&overlay, "user/GET.json", &user("overlay"));
    let out = gh(&s.bin, &[&overlay, &s.set], Some(&s.log))
        .args(["api", "/user"])
        .output()
        .unwrap();
    assert_eq!(stdout(&out), r#"{"login":"overlay"}"#);
    assert_eq!(read_log(&s.log)[0]["fixture"], "overlay/user/GET.json");
}

#[test]
fn base_fixtures_answer_the_python_requests() {
    let tmp = TempDir::new("base");
    let bin = install_gh(tmp.path());
    let base = common::repo_root().join("parity/fixtures/base");
    let run = |args: &[&str]| gh(&bin, &[&base], None).args(args).output().unwrap();

    let repos = run(&[
        "api",
        "-i",
        "-X",
        "GET",
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "Cache-Control: max-age=600",
        "-F",
        "type=all",
        "-F",
        "direction=asc",
        "-F",
        "sort=full_name",
        "-F",
        "page=1",
        "-F",
        "per_page=50",
        "/user/repos",
    ]);
    assert_eq!(repos.status.code(), Some(0));
    let text = stdout(&repos);
    let body = text.split("\r\n\r\n").nth(1).unwrap();
    let repos: Vec<Value> = serde_json::from_str(body).unwrap();
    assert_eq!(repos.len(), 3);
    assert!(repos.iter().all(|r| r["private"] == false));

    let user: Value = serde_json::from_str(&stdout(&run(&["api", "/user"]))).unwrap();
    assert_eq!(
        (user["login"].as_str(), user["id"].as_u64()),
        (Some("parity-bot"), Some(1_000_001))
    );

    let notifications = run(&[
        "api",
        "-i",
        "-X",
        "GET",
        "-F",
        "all=false",
        "-F",
        "page=1",
        "-F",
        "per_page=50",
        "/notifications",
    ]);
    assert_eq!(notifications.status.code(), Some(0));
    assert!(stdout(&notifications).ends_with("\r\n\r\n[]"));
}
