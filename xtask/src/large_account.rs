//! The `large-account` fixture set (#325): 1,000 repos for `parity-bot` over 20 `GET /user/repos` pages of 50, for
//! the Rust-only `large-account` scenario. Generated here rather than written by hand; `cargo xtask
//! large-account-fixtures` writes it under `parity/fixtures/large-account/`, and a test checks the committed files
//! match.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

/// The set's directory, from the repo root.
pub const DIR: &str = "parity/fixtures/large-account";

/// How many repos the account has.
pub const REPOS: usize = 1000;

/// Repos per page: what both apps ask for.
const PER_PAGE: usize = 50;

/// Repo `i`'s name: zero-padded, so the names sort in number order.
pub fn repo_name(i: usize) -> String {
    format!("large-{i:04}")
}

fn repo(i: usize) -> Value {
    let name = repo_name(i);
    let full_name = format!("parity-bot/{name}");
    json!({
        "id": 5_000_000 + i,
        "node_id": format!("R_large{i:04}"),
        "name": name,
        "full_name": full_name,
        "private": false,
        "owner": {
            "login": "parity-bot",
            "id": 1_000_001,
            "node_id": "U_kgDOAA9CQQ",
            "avatar_url": "https://avatars.githubusercontent.com/u/0?v=4",
            "url": "https://api.github.com/users/parity-bot",
            "html_url": "https://github.com/parity-bot",
            "type": "User",
            "site_admin": false
        },
        "html_url": format!("https://github.com/{full_name}"),
        "description": format!("Large account repository {} of {REPOS}", i + 1),
        "fork": false,
        "url": format!("https://api.github.com/repos/{full_name}"),
        "created_at": "2024-02-01T10:00:00Z",
        "updated_at": "2026-09-30T08:00:00Z",
        "pushed_at": "2026-09-29T21:15:00Z",
        "default_branch": "main",
        "archived": false,
        "visibility": "public",
        "permissions": {"admin": true, "maintain": true, "push": true, "triage": true, "pull": true}
    })
}

/// The `Link` header GitHub sends with page `page` of `pages`.
fn link(page: usize, pages: usize) -> String {
    let url = |n: usize| {
        format!(
            "<https://api.github.com/user/repos?type=all&direction=asc&sort=full_name&per_page={PER_PAGE}&page={n}>"
        )
    };
    let mut links = Vec::new();
    if page > 1 {
        links.push(format!("{}; rel=\"prev\"", url(page - 1)));
    }
    if page < pages {
        links.push(format!("{}; rel=\"next\"", url(page + 1)));
        links.push(format!("{}; rel=\"last\"", url(pages)));
    }
    if page > 1 {
        links.push(format!("{}; rel=\"first\"", url(1)));
    }
    links.join(", ")
}

/// Every file of the set: its path under [`DIR`] and its text.
pub fn files() -> Vec<(PathBuf, String)> {
    let pages = REPOS.div_ceil(PER_PAGE);
    (1..=pages)
        .map(|page| {
            let first = (page - 1) * PER_PAGE;
            let body: Vec<Value> = (first..(first + PER_PAGE).min(REPOS)).map(repo).collect();
            let fixture = json!({
                "version": 1,
                "request": {
                    "method": "GET",
                    "path": "/user/repos",
                    "query": {
                        "direction": "asc",
                        "page": page.to_string(),
                        "per_page": PER_PAGE.to_string(),
                        "sort": "full_name",
                        "type": "all"
                    }
                },
                "response": {
                    "status": 200,
                    "headers": {
                        "Content-Type": "application/json; charset=utf-8",
                        "Link": link(page, pages)
                    },
                    "body": body
                }
            });
            let path = PathBuf::from(format!(
                "user/repos/GET.direction-asc_page-{page}_per_page-{PER_PAGE}_sort-full_name_type-all.json"
            ));
            // `to_string_pretty` can't fail on a `Value`.
            let text = serde_json::to_string_pretty(&fixture).unwrap_or_default() + "\n";
            (path, text)
        })
        .collect()
}

/// Writes the set under `repo`, replacing what's there.
pub fn write(repo: &Path) -> std::io::Result<()> {
    let dir = repo.join(DIR);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    for (path, text) in files() {
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, text)?;
    }
    Ok(())
}

pub fn run(repo: &Path) -> ExitCode {
    match write(repo) {
        Ok(()) => {
            println!("wrote {}", repo.join(DIR).display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("xtask large-account-fixtures: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_set_is_what_the_generator_writes() {
        let dir = crate::parity::cli::repo_root().join(DIR);
        let expected = files();
        for (path, text) in &expected {
            let committed = std::fs::read_to_string(dir.join(path))
                .unwrap_or_else(|e| panic!("{}: {e}; run `cargo xtask large-account-fixtures`", path.display()));
            assert!(
                committed == *text,
                "{} is stale; run `cargo xtask large-account-fixtures`",
                path.display()
            );
        }
        let mut committed = 0;
        let mut stack = vec![dir];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    committed += 1;
                }
            }
        }
        assert_eq!(
            committed,
            expected.len(),
            "files in {DIR} that the generator doesn't write"
        );
    }

    #[test]
    fn the_pages_hold_every_repo_once_and_link_to_the_next() {
        let files = files();
        assert_eq!(files.len(), 20);
        let mut names = Vec::new();
        for (index, (_, text)) in files.iter().enumerate() {
            let fixture: Value = serde_json::from_str(text).unwrap();
            let link = fixture["response"]["headers"]["Link"].as_str().unwrap();
            assert_eq!(link.contains("rel=\"next\""), index + 1 < files.len(), "{link}");
            for repo in fixture["response"]["body"].as_array().unwrap() {
                names.push(repo["name"].as_str().unwrap().to_owned());
            }
        }
        assert_eq!(names, (0..REPOS).map(repo_name).collect::<Vec<_>>());
    }
}
