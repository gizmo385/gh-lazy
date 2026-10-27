"""Generates the table cache goldens for the Rust `lazy-github-core` cache tests, with the Python app's own cache code.

Run from the repository root with:

    uv run python crates/lazy-github-core/tests/fixtures/cache/generate.py

The models are built from the parity fixtures the way the Python app builds them from the API, and written with
`save_models_to_cache`:

- repos.json: `Repository(**r)` for each repo of base's `/user/repos` page.
- parity-bot_cache-fixture_pull_requests.json and parity-bot_cache-fixture_issues.json: table-caches' first
  `/repos/parity-bot/cache-fixture/issues` page through `PartialPullRequest(**item, repo=repo)` (items with `draft`) and
  `Issue(**item, repo=repo)` (the rest), as `list_issues` splits them, `repo` being cache-fixture from table-caches'
  `/user/repos` page.
- parity-bot_cache-fixture_workflow_runs.json: `WorkflowRun(**run)` for each run of table-caches' first
  `/repos/parity-bot/cache-fixture/actions/runs` page, as `list_workflow_runs` builds them.

`--out DIR` writes them to DIR instead of this directory; the `cache_goldens_are_fresh` test uses it.
"""

import argparse
import json
from pathlib import Path
from typing import Any

from lazy_github.lib.cache import save_models_to_cache
from lazy_github.lib.config import Config
from lazy_github.lib.context import LazyGithubContext
from lazy_github.models.github import Issue, PartialPullRequest, Repository, WorkflowRun

HERE = Path(__file__).parent
FIXTURES = HERE.parents[4] / "parity" / "fixtures"
USER_REPOS = "user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json"
CACHE_FIXTURE_ISSUES = "repos/parity-bot/cache-fixture/issues/GET.page-1_per_page-30_state-all.json"
CACHE_FIXTURE_RUNS = "repos/parity-bot/cache-fixture/actions/runs/GET.page-1_per_page-30.json"


def body(fixture: str, path: str) -> Any:
    return json.loads((FIXTURES / fixture / path).read_text())["response"]["body"]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=HERE, help="directory to write the goldens to")
    args = parser.parse_args()

    # A config of its own, so the user's config and log file are never touched, caching into `--out`.
    LazyGithubContext._config = Config.model_validate({"cache": {"cache_directory": str(args.out)}})

    repos = [Repository(**r) for r in body("base", USER_REPOS)]
    save_models_to_cache(None, "repos", repos)

    (repo,) = [Repository(**r) for r in body("table-caches", USER_REPOS) if r["full_name"] == "parity-bot/cache-fixture"]
    pull_requests: list[PartialPullRequest] = []
    issues: list[Issue] = []
    for item in body("table-caches", CACHE_FIXTURE_ISSUES):
        if "draft" in item:
            pull_requests.append(PartialPullRequest(**item, repo=repo))
        else:
            issues.append(Issue(**item, repo=repo))
    save_models_to_cache(repo, "pull_requests", pull_requests)
    save_models_to_cache(repo, "issues", issues)

    runs = [WorkflowRun(**run) for run in body("table-caches", CACHE_FIXTURE_RUNS)["workflow_runs"]]
    save_models_to_cache(repo, "workflow_runs", runs)


if __name__ == "__main__":
    main()
