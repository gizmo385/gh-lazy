import json
from dataclasses import dataclass, asdict
from pathlib import Path

from lazy_github.lib.context import LazyGithubContext
from lazy_github.lib.logging import lg


@dataclass
class PullRequestDraft:
    repo_full_name: str
    title: str
    description: str
    base_ref: str
    head_ref: str
    is_draft: bool
    reviewers: list[str]


def get_draft_path(repo_full_name: str) -> Path:
    """Returns the path to the draft file for a given repository."""
    safe_name = repo_full_name.replace("/", "_")
    return LazyGithubContext.config.cache.cache_directory / "pr_drafts" / f"{safe_name}.json"


def load_pr_draft(repo_full_name: str) -> PullRequestDraft | None:
    """Load a PR draft for the given repository, returning None if not found or corrupt."""
    draft_path = get_draft_path(repo_full_name)
    if not draft_path.exists():
        return None

    try:
        data = json.loads(draft_path.read_text())
        return PullRequestDraft(**data)
    except json.JSONDecodeError as e:
        lg.warning(f"Failed to parse PR draft file '{draft_path}' as valid JSON: {e}. Ignoring draft.")
        return None
    except (TypeError, KeyError) as e:
        lg.warning(f"PR draft file '{draft_path}' has invalid structure: {e}. Ignoring draft.")
        return None
    except Exception as e:
        lg.warning(f"Unexpected error loading PR draft from '{draft_path}': {e}. Ignoring draft.")
        return None


def save_pr_draft(draft: PullRequestDraft) -> None:
    """Save a PR draft to disk."""
    draft_path = get_draft_path(draft.repo_full_name)
    try:
        draft_path.parent.mkdir(parents=True, exist_ok=True)
        draft_path.write_text(json.dumps(asdict(draft), indent=2))
        lg.debug(f"Saved PR draft to {draft_path}")
    except Exception as e:
        lg.warning(f"Failed to save PR draft to '{draft_path}': {e}")


def clear_pr_draft(repo_full_name: str) -> None:
    """Delete the draft file for the given repository."""
    draft_path = get_draft_path(repo_full_name)
    try:
        if draft_path.exists():
            draft_path.unlink()
            lg.debug(f"Cleared PR draft at {draft_path}")
    except Exception as e:
        lg.warning(f"Failed to clear PR draft at '{draft_path}': {e}")
