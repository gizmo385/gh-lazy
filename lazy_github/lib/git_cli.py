import re
from subprocess import DEVNULL, PIPE, SubprocessError, check_output, run

from lazy_github.lib.logging import lg

# Regex designed to match git@github.com:gizmo385/lazy-github.git:
# ".+:"         Match everything to the first colon
# "([^\/]+)"    Match everything until the forward slash, which should be owner
# "\/"          Match the forward slash
# "([^.]+)"     Match everything until the period, which should be the repo name
# ".git"        Match the .git suffix
_SSH_GIT_REMOTE_REGEX = re.compile(r".+:([^\/]+)\/([^.]+)(?:.git)?")

# Regex designed to match something like "https://github.com/gizmo385/gh-lazy.git"
# "^https:\/\/[^.]+[^\/]+"  Match the base URL until the first non-prefix slash (eg: https://github.com)
# "([^\/]+)"                Match the username (eg: gizmo385)
# "([^\/]+)$"               Match the end of the URL (eg: gh-lazy.git)
_HTTPS_GIT_REMOTE_REGEX = re.compile(r"^https:\/\/[^.]+[^\/]+\/([^\/]+)\/([^\/]+)$")


def current_local_repo_full_name(remote: str = "origin") -> str | None:
    """Returns the owner/name associated with the remote of the git repo in the current working directory."""
    try:
        output = check_output(["git", "remote", "get-url", remote], stderr=DEVNULL).decode().strip()
    except SubprocessError:
        return None

    if matches := re.match(_SSH_GIT_REMOTE_REGEX, output) or re.match(_HTTPS_GIT_REMOTE_REGEX, output):
        owner, name = matches.groups()
        return f"{owner}/{name}"


def current_local_repo_matches_selected_repo(remote: str = "origin") -> bool:
    """Checks to see if the current repo and the repo selected in LazyGithub are the same"""
    from lazy_github.lib.context import LazyGithubContext

    if local_repo := current_local_repo_full_name(remote):
        return bool(LazyGithubContext.current_repo) and local_repo == LazyGithubContext.current_repo.full_name
    else:
        return False


def current_local_branch_name() -> str | None:
    """Returns the name of the current branch for the git repo in the current working directory."""
    try:
        return check_output(["git", "rev-parse", "--abbrev-ref", "HEAD"], stderr=DEVNULL).decode().strip()
    except SubprocessError:
        return None


def current_local_commit() -> str | None:
    """Returns the commit sha for the git repo in the current working directory"""
    try:
        return check_output(["git", "rev-parse", "HEAD"], stderr=DEVNULL).decode().strip()
    except SubprocessError:
        return None


def does_branch_exist_on_remote(branch: str, remote: str = "origin") -> bool:
    try:
        return bool(check_output(["git", "ls-remote", remote, branch]))
    except SubprocessError:
        return False


def does_branch_have_configured_upstream(branch: str) -> bool:
    """Checks to see if the specified branch is configured with an upstream"""
    try:
        return run(["git", "config", "--get", f"branch.{branch}.merge"]).returncode == 0
    except SubprocessError:
        return False


def push_branch_to_remote(branch: str, remote: str = "origin") -> bool:
    """
    If the current local repo and the selected repo are the same, pushes the current branch to the remote and sets
    the branches upstream to track the new remote
    """
    if not current_local_repo_matches_selected_repo(remote):
        return False

    try:
        result = run(["git", "push", "--set-upstream", remote, f"HEAD:{branch}"], stdout=PIPE, stderr=PIPE)
        return result.returncode == 0
    except SubprocessError:
        lg.exception("Error pushing branch to remote")
        return False
