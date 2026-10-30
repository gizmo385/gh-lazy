"""Dumps what the Python app's GitHub link patterns make of a corpus of texts, for the Rust parser's golden test:
github_url_patterns.json.

For each text it records `GithubLinkValidator`'s answer and which link `resolve_github_url` would look up with and
without `accept_repo` (the paste-link dialog accepts repository links, link clicks don't), in its order: user, pull
request, issue, repository. `crates/lazy-github-tui/tests/github_url_patterns.rs` checks `core::links` against it.

Run from the repository root:

    uv run python crates/lazy-github-tui/tests/fixtures/generate_github_url_patterns.py

`--out DIR` writes it to DIR instead of this directory; the `github_url_patterns_golden_is_fresh` test uses it.
"""

import argparse
import json
from pathlib import Path
from typing import Any

from lazy_github.ui.screens.link_paste import (
    ISSUE_PATTERN,
    PR_PATTERN,
    REPO_PATTERN,
    USER_PATTERN,
    GithubLinkValidator,
)

CORPUS = [
    "",
    "not a link",
    "https://github.com/parity-bot",
    "github.com/parity-bot",
    "https://github.com/parity-bot/",
    "github.com/parity-bot/",
    "https://github.com/parity-bot\n",
    "https://github.com/a",
    "https://github.com/a-b-c",
    "https://github.com/-x",
    "https://github.com/x-",
    "https://github.com/x_y",
    "https://example.com/user",
    "example.com/user",
    "http://github.com/parity-bot",
    "https://GitHub.com/parity-bot",
    "https://github.com/o/r",
    "github.com/o/r",
    "https://github.com/o/r/",
    "https://github.com/o/r/pull/5",
    "github.com/o/r/pull/5",
    "https://github.com/o/r/pull/5/files",
    "https://github.com/o/r/pull/5#discussion_r1",
    "https://github.com/o/r/pull/",
    "https://github.com/o/r/pull/abc",
    "https://github.com/o/r/pull/5abc",
    "https://github.com/o/r/pulls",
    "https://github.com/o/r/pull/99999999999999999999999",
    "https://github.com/o/r/issues/12",
    "github.com/o/r/issues/12",
    "https://github.com/o/r/issues/5#issuecomment-1",
    "https://github.com/o/r/issues",
    "https://github.com/o/r/issues/new",
    "https://github.com/o/r/actions/runs/1",
    "https://github.com/o/r/blob/main/README.md",
    "https://github.com/o/r?tab=readme",
    "https://github.com/o/r#readme",
    "http://github.com/o/r",
    "http://github.com/o/r/pull/5",
    "https://gitlab.com/o/r/pull/5",
    "https://ghe.example.com/o/r/issues/3",
    "https://github.com/o",
    "o/r",
    "o/r/pull/5",
    "https:/github.com/o/r",
    "https://github.com//r",
    "https://github.com/o r/name with spaces",
    "https://github.com/o/r/pull/5\n",
    "  https://github.com/o/r",
    "https://",
    "/o/r",
]


def link(text: str, accept_repo: bool) -> dict[str, Any] | None:
    """What `resolve_github_url` would look up for `text`, before any request."""
    if match := USER_PATTERN.match(text):
        return {"kind": "user", "login": match.group("login")}
    if match := PR_PATTERN.match(text):
        return {"kind": "pull_request", **match.groupdict(), "number": int(match.group("pull_request"))}
    if match := ISSUE_PATTERN.match(text):
        return {"kind": "issue", **match.groupdict(), "number": int(match.group("issue"))}
    if accept_repo and (match := REPO_PATTERN.match(text)):
        return {"kind": "repository", **match.groupdict()}
    return None


def clean(found: dict[str, Any] | None) -> dict[str, Any] | None:
    if found is None:
        return None
    return {key: value for key, value in found.items() if key not in ("pull_request", "issue")}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path(__file__).parent, help="directory to write the golden to")
    args = parser.parse_args()
    validator = GithubLinkValidator()
    golden = [
        {
            "text": text,
            "valid": validator.validate(text).is_valid,
            "accept_repo": clean(link(text, accept_repo=True)),
            "reject_repo": clean(link(text, accept_repo=False)),
        }
        for text in CORPUS
    ]
    (args.out / "github_url_patterns.json").write_text(json.dumps(golden, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
