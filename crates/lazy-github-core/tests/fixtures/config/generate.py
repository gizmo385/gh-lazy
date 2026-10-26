"""Generates and checks the config fixtures for the Rust `lazy-github-core` config tests.

Run from the repository root with:

    HOME=/home/lazy uv run python crates/lazy-github-core/tests/fixtures/config/generate.py

`python-default.json` and `python-full.json` are written by the Python app. `python-parsing.json` records, for each
field and each form its type allows (`PARSING_CASES`), what the Python app loads and saves, or where it rejects the
file. The other fixtures are written by hand; this script only checks that the Python app loads them, so they stay
valid Python configs.

`--out DIR` writes the generated fixtures to DIR instead of this directory; the hand-written ones are still read from
here. The `config_goldens_are_fresh` test uses it.
"""

import argparse
import json
from datetime import datetime
from enum import StrEnum
from pathlib import Path
from typing import Any

from pydantic import ValidationError

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


BOOL_FIELDS = [
    "appearance.show_command_log",
    "appearance.show_workflows",
    "appearance.show_issues",
    "appearance.show_pull_requests",
    "notifications.enabled",
    "notifications.mark_notification_as_read_when_selected",
    "pull_requests.preload_pull_request_for_current_commit",
    "core.first_start",
]
BOOL_INPUTS = [
    True, False, "true", "FaLsE", "yes", "no", "On", "off", "t", "f", "y", "N", "1", "0",
    1, 0, 1.0, 0.0, 2, "maybe", "", " true ", "None", None,
]  # fmt: skip

INT_FIELDS = [
    "cache.auth_cache_duration",
    "cache.default_ttl",
    "cache.list_repos_ttl",
    "cache.list_issues_ttl",
    "core.logfile_max_bytes",
    "core.logfile_count",
]
INT_INPUTS = [
    600, 600.0, "600", "600.0", " 600 ", "+5", "1_000", True, False, -1, "-5", -1.0,
    600.5, "600.5", "1e3", "ten", "", None,
]  # fmt: skip

ENUM_FIELDS: list[tuple[str, type[StrEnum]]] = [
    ("pull_requests.state_filter", IssueStateFilter),
    ("pull_requests.owner_filter", IssueOwnerFilter),
    ("pull_requests.preferred_merge_method", MergeMethod),
    ("issues.state_filter", IssueStateFilter),
    ("issues.owner_filter", IssueOwnerFilter),
]


def enum_inputs(enum: type[StrEnum]) -> list[Any]:
    """Each valid value, the first one upper-cased and capitalized, then values no enum accepts."""
    values = [member.value for member in enum]
    return [*values, values[0].upper(), values[0].capitalize(), "bogus", "", 1, None]


LIST_FIELDS = [
    "repositories.additional_repos_to_track",
    "repositories.favorites",
    "pull_requests.additional_suggested_pr_reviewers",
]
LIST_INPUTS = [[], ["a/b"], ["a/b", "a/b"], "a/b, c/d", [1], None]

# Relative or absolute, never resolved against HOME, so the cases don't depend on it.
PATH_FIELDS = ["cache.cache_directory", "core.logfile_path", "pull_requests.pull_request_template"]
PATH_INPUTS = ["/abs/x", "~/x", "rel/x", "", 5, None]

THEME_INPUTS = ["nord", "textual-dark", "ansi-dark", "Nord", "not-a-real-theme", "", 5, None, {"name": "nord"}]

DATETIME_INPUTS = [
    "2026-01-02T03:04:05",
    "2026-01-02T03:04:05.000007",
    "2026-01-02",
    "2026-01-02 03:04:05",
    "2026-01-02T03:04:05+02:00",
    "None",
    "yesterday",
    "",
    0,
    None,
]

OVERRIDES_INPUTS = [{"app.quit": "ctrl+q"}, {"app.quit": 1}, {"nope": "x"}, []]

# Every (field, input) pair, in the order `python-parsing.json` lists them.
PARSING_CASES: list[tuple[str, Any]] = [
    *((field, value) for field in BOOL_FIELDS for value in BOOL_INPUTS),
    *((field, value) for field in INT_FIELDS for value in INT_INPUTS),
    *((field, value) for field, enum in ENUM_FIELDS for value in enum_inputs(enum)),
    *((field, value) for field in LIST_FIELDS for value in LIST_INPUTS),
    *((field, value) for field in PATH_FIELDS for value in PATH_INPUTS),
    *(("appearance.theme", value) for value in THEME_INPUTS),
    *(("cache.auth_last_checked", value) for value in DATETIME_INPUTS),
    *(("bindings.overrides", value) for value in OVERRIDES_INPUTS),
]


def parse_case(field: str, value: Any) -> dict[str, Any]:
    """What loading `{section: {key: value}}` does: the value a save writes back (`ok`), pydantic's first error
    location (`error`), or the type of an exception that escapes validation, which the app doesn't catch
    (`exception`)."""
    section, key = field.split(".")
    case: dict[str, Any] = {"field": field, "input": value}
    try:
        # The same validation as `Config.load_config`; `model_dump(mode="json")` is what `save` writes.
        saved = Config.model_validate({section: {key: value}}).model_dump(mode="json")
    except ValidationError as e:
        return case | {"result": "error", "loc": ".".join(str(part) for part in e.errors()[0]["loc"])}
    except Exception as e:
        return case | {"result": "exception", "exception": type(e).__name__}
    return case | {"result": "ok", "saved": saved[section][key]}


def parsing_golden() -> str:
    # One case per line, so a diff shows which cases changed.
    lines = ",\n".join("    " + json.dumps(parse_case(field, value)) for field, value in PARSING_CASES)
    return f"[\n{lines}\n]\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=HERE, help="directory to write the generated fixtures to")
    args = parser.parse_args()

    (args.out / "python-default.json").write_text(Config().model_dump_json(indent=4))
    (args.out / "python-full.json").write_text(full_config().model_dump_json(indent=4))
    (args.out / "python-parsing.json").write_text(parsing_golden())
    for name in HAND_WRITTEN:
        Config(**json.loads((HERE / name).read_text()))
        print(f"{name}: loads in Python")


if __name__ == "__main__":
    main()
