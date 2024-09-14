import subprocess
import re

# Regex designed to match git@github.com:gizmo385/lazy-github.git:
# ".+:"         Match everything to the first colon
# "([^\/]+)"    Match everything until the forward slash, which should be owner
# "\/"          Match the forward slash
# "([^.]+)"     Match everything until the period, which should be the repo name
# ".git"        Match the .git suffix
_GIT_REMOTE_REGEX = re.compile(r".+:([^\/]+)\/([^.]+).git")


def current_local_repo_full_name(remote: str = "origin") -> str | None:
    """Returns the owner/name associated with the remote of the git repo in the current working directory."""
    try:
        output = subprocess.check_output(["git", "remote", "get-url", remote]).decode().strip()
    except subprocess.SubprocessError:
        return None

    if matches := re.match(_GIT_REMOTE_REGEX, output):
        owner, name = matches.groups()
        return f"{owner}/{name}"


def current_local_branch_name() -> str | None:
    """Returns the name of the current branch for the git repo in the current working directory."""
    try:
        return subprocess.check_output(["git", "rev-parse", "--abbrev-ref", "HEAD"]).decode().strip()
    except subprocess.SubprocessError:
        return None


print(current_local_branch_name())