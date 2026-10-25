"""Generates and checks the config fixtures for the Rust `lazy-github-core` config tests.

Run from the repository root with:

    HOME=/home/lazy uv run python crates/lazy-github-core/tests/fixtures/config/generate.py

`python-default.json` and `python-full.json` are written by the Python app. The other fixtures are written by hand;
this script only checks that the Python app loads them, so they stay valid Python configs.
"""

import json
from datetime import datetime
from pathlib import Path

from lazy_github.lib.config import Config, MergeMethod
from lazy_github.lib.constants import IssueOwnerFilter, IssueStateFilter
from lazy_github.lib.github.backends.protocol import BackendType

HERE = Path(__file__).parent
HAND_WRITTEN = ["python-partial.json", "unknown-fields.json", "lax-values.json"]


def full_config() -> Config:
    config = Config.model_validate({"appearance": {"theme": "nord"}})
    config.appearance.show_command_log = True
    config.appearance.show_workflows = False
    config.appearance.show_issues = False
    config.appearance.show_pull_requests = False
    config.notifications.enabled = True
    config.notifications.mark_notification_as_read_when_selected = False
    config.bindings.overrides = {"app.quit": "ctrl+q", "common.table.select": "enter,space"}
    config.repositories.additional_repos_to_track = ["Textualize/textual", "org/repo.with-chars_1"]
    config.repositories.favorites = ["gizmo385/gh-lazy", "gizmo385/dotfiles"]
    config.pull_requests.state_filter = IssueStateFilter.OPEN
    config.pull_requests.owner_filter = IssueOwnerFilter.MINE
    config.pull_requests.preferred_merge_method = MergeMethod.REBASE
    config.pull_requests.additional_suggested_pr_reviewers = ["octocat", "hubot"]
    config.pull_requests.preload_pull_request_for_current_commit = True
    config.pull_requests.pull_request_template = Path("/home/lazy/templates/pr.md")
    config.issues.state_filter = IssueStateFilter.OPEN
    config.issues.owner_filter = IssueOwnerFilter.MINE
    config.cache.auth_cache_duration = 7200
    config.cache.auth_last_checked = datetime(2026, 10, 24, 18, 30, 5, 123456)
    config.cache.cache_directory = Path("/var/cache/lazy-github")
    config.cache.default_ttl = 300
    config.cache.list_repos_ttl = 43200
    config.cache.list_issues_ttl = 1800
    config.core.first_start = False
    config.core.logfile_path = Path("/var/log/lazy-github/app.log")
    config.core.logfile_max_bytes = 1000000
    config.core.logfile_count = 3
    config.api.base_url = "https://github.example.com/api/v3"
    config.api.client_type = BackendType.GITHUB_CLI
    return config


def main() -> None:
    (HERE / "python-default.json").write_text(Config().model_dump_json(indent=4))
    (HERE / "python-full.json").write_text(full_config().model_dump_json(indent=4))
    for name in HAND_WRITTEN:
        Config(**json.loads((HERE / name).read_text()))
        print(f"{name}: loads in Python")


if __name__ == "__main__":
    main()
