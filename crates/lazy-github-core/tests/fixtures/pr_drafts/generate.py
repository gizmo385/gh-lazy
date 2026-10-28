"""Generates the PR draft goldens for the Rust `lazy-github-core` drafts tests, with the Python app's own draft code.

Run from the repository root with:

    uv run python crates/lazy-github-core/tests/fixtures/pr_drafts/generate.py

Each draft is written with `save_pr_draft`, into `pr_drafts/` under the cache directory, as Create PR saves it:

- parity-bot_pr-details.json: a title, a two-paragraph description, base `main`, Compare `feature/restored`, Draft on
  and one reviewer, the draft the `pr-draft` parity seed holds.
- parity-bot_unicode.json: non-ASCII text (which Python escapes as `\\uXXXX`), no reviewers and Draft off.

`--out DIR` makes DIR the cache directory instead of this file's parent; the `pr_drafts_goldens_are_fresh` test uses
it.
"""

import argparse
from pathlib import Path

from lazy_github.lib.config import Config
from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.pr_drafts import PullRequestDraft, save_pr_draft

HERE = Path(__file__).parent


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=HERE.parent, help="cache directory to write the goldens under")
    args = parser.parse_args()

    # A config of its own, so the user's config and log file are never touched, caching into `--out`.
    LazyGithubContext._config = Config.model_validate({"cache": {"cache_directory": str(args.out)}})

    save_pr_draft(
        PullRequestDraft(
            repo_full_name="parity-bot/pr-details",
            title="Restored draft title",
            description="Restored draft body.\n\nSecond paragraph.",
            base_ref="main",
            head_ref="feature/restored",
            is_draft=True,
            reviewers=["parity-friend"],
        )
    )
    save_pr_draft(
        PullRequestDraft(
            repo_full_name="parity-bot/unicode",
            title="Ünïcödé títle — ✓",
            description="Naïve café notes: 日本語 and emoji 🎉.",
            base_ref="main",
            head_ref="feature/ünï",
            is_draft=False,
            reviewers=[],
        )
    )


if __name__ == "__main__":
    main()
