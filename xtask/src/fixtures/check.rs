//! `cargo xtask check-fixtures`: the scrub rules, read-only, over every committed fixture (hand-written ones too).
//! No network. `cargo test` runs it through the test below, so CI enforces it.

use std::path::{Path, PathBuf};

use fake_gh::store::scan_set;

use super::scrub::Rules;

/// Checks every set (each directory) under `root`, usually `parity/fixtures`. Returns one line per finding.
pub fn check_root(root: &Path) -> Vec<String> {
    let mut sets: Vec<PathBuf> = match std::fs::read_dir(root) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(e) => return vec![format!("can't read {}: {e}", root.display())],
    };
    sets.sort();
    sets.iter().flat_map(|set| check_set(set)).collect()
}

/// Checks one set directory.
pub fn check_set(set_dir: &Path) -> Vec<String> {
    let (fixtures, errors) = scan_set(set_dir, 0);
    let mut problems: Vec<String> = errors.iter().map(ToString::to_string).collect();
    for f in &fixtures {
        let body_file = match f.body_file_path().map(std::fs::read) {
            None => None,
            Some(Ok(bytes)) => Some(bytes),
            Some(Err(e)) => {
                problems.push(format!("{}: can't read body_file: {e}", f.display));
                None
            }
        };
        problems.extend(
            Rules::get()
                .check_fixture(&f.fixture, body_file.as_deref())
                .iter()
                .map(|finding| format!("{}: {finding}", f.display)),
        );
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::repo_root;
    use crate::fixtures::test_support::{TempDir, write};

    #[test]
    fn committed_fixtures_pass_the_scrub_check() {
        let root = repo_root().join("parity").join("fixtures");
        let problems = check_root(&root);
        assert!(problems.is_empty(), "scrub check findings:\n{}", problems.join("\n"));
    }

    fn fixture_with_body(body: &serde_json::Value) -> String {
        serde_json::json!({
            "version": 1,
            "request": {"method": "GET", "path": "/users/x"},
            "response": {"status": 200, "body": body}
        })
        .to_string()
    }

    #[test]
    fn a_real_looking_email_fails_the_check() {
        let tmp = TempDir::new("check");
        let set = tmp.path().join("base");
        // Built at runtime so the repo holds no real-looking address.
        let email = format!("user@{}.com", "gmail");
        write(
            &set,
            "users/x/GET.json",
            &fixture_with_body(&serde_json::json!({"login": "x", "email": email})),
        );
        assert_eq!(
            check_root(tmp.path()),
            [r#"base/users/x/GET.json: JSON pointer "/email": email should be user@example.com"#]
        );

        write(
            &set,
            "users/x/GET.json",
            &serde_json::json!({
                "version": 1,
                "request": {"method": "GET", "path": "/users/x"},
                "response": {"status": 200, "body_file": "GET.body.txt"}
            })
            .to_string(),
        );
        write(&set, "users/x/GET.body.txt", &format!("contact: {email}\n"));
        assert_eq!(
            check_set(&set),
            ["base/users/x/GET.json: byte offset 9: email should be user@example.com"]
        );
    }

    #[test]
    fn secrets_private_repos_and_headers_fail_the_check() {
        let tmp = TempDir::new("check-secrets");
        let set = tmp.path().join("base");
        let token = format!("gh{}_{}", 'p', "a".repeat(36));
        write(
            &set,
            "users/x/GET.json",
            &serde_json::json!({
                "version": 1,
                "request": {"method": "GET", "path": "/users/x"},
                "response": {"status": 200, "headers": {"X-Oauth-Scopes": "repo"},
                             "body": [{"private": true, "bio": token}]}
            })
            .to_string(),
        );
        let problems = check_set(&set);
        assert_eq!(
            problems,
            [
                "base/users/x/GET.json: header X-Oauth-Scopes: header not in the allow-list (Content-Type, Link, \
                 Location)",
                r#"base/users/x/GET.json: JSON pointer "/0": object has "private": true"#,
                r#"base/users/x/GET.json: JSON pointer "/0/bio": looks like a secret (GitHub token)"#,
            ]
        );
    }

    #[test]
    fn placeholders_and_binary_body_files_pass() {
        let tmp = TempDir::new("check-clean");
        let set = tmp.path().join("base");
        write(
            &set,
            "users/x/GET.json",
            &fixture_with_body(&serde_json::json!({
                "email": "user@example.com",
                "author_email": null,
                "avatar_url": super::super::scrub::PLACEHOLDER_AVATAR,
                "gravatar_id": "",
                "bio": "noreply: 1+x@users.noreply.github.com",
            })),
        );
        let binary = r#"{"version":1,"request":{"method":"GET","path":"/repos/o/r/actions/runs/1/logs"},"response":{"status":200,"body_file":"GET.body.bin"}}"#;
        write(&set, "repos/o/r/actions/runs/1/logs/GET.json", binary);
        let mut zip = b"PK\x03\x04".to_vec();
        zip.extend_from_slice(&[0xff, 0xfe]);
        zip.extend_from_slice(b"x@corp.test");
        std::fs::write(set.join("repos/o/r/actions/runs/1/logs/GET.body.bin"), zip).unwrap();
        assert_eq!(check_set(&set), Vec::<String>::new());
    }
}
