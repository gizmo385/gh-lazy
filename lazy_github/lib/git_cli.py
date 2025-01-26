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
    try:
        result = run(["git", "push", "--set-upstream", remote, f"HEAD:{branch}"], stdout=PIPE, stderr=PIPE)
        return result.returncode == 0
    except SubprocessError:
        lg.exception("Error pushing branch to remote")
        return False
