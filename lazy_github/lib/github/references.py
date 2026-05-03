"""Expand GitHub-flavored issue/PR references in Markdown bodies into real links.

GitHub auto-links references like `#123`, `owner/repo#123`, and
`https://github.com/owner/repo/pull/123` in rendered issue/PR/comment bodies.
The raw Markdown source does not contain those links, so the Textual Markdown
widget never sees them as clickable. We pre-process the body to replace bare
references with proper Markdown links so the LinkClicked handler can intercept.
"""

import re

from lazy_github.models.github import Repository

# Matches `#123` and `owner/repo#123`. The owner/repo prefix is optional.
# Negative lookbehind on `\w`, `/`, `&`, and `#` keeps us from matching
# inside identifiers, paths, HTML entities, or `##headings`.
_REFERENCE_RE = re.compile(
    r"(?<![\w/&#])"
    r"(?:(?P<owner>[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)/(?P<repo>[A-Za-z0-9._-]+))?"
    r"#(?P<number>\d+)\b"
)

_FENCE_RE = re.compile(r"^\s*(```|~~~)")


def _expand_segment(segment: str, owner: str, repo: str) -> str:
    def repl(match: re.Match[str]) -> str:
        ref_owner = match.group("owner") or owner
        ref_repo = match.group("repo") or repo
        number = match.group("number")
        url = f"https://github.com/{ref_owner}/{ref_repo}/issues/{number}"
        text = match.group(0)
        return f"[{text}]({url})"

    return _REFERENCE_RE.sub(repl, segment)


def expand_issue_references(body: str | None, repo: Repository) -> str:
    """Rewrite `#N` and `owner/repo#N` references into Markdown links.

    Skips fenced code blocks and inline backtick spans so we don't munge code.
    """
    if not body:
        return body or ""

    owner, _, repo_name = repo.full_name.partition("/")
    if not owner or not repo_name:
        return body

    out_lines: list[str] = []
    in_fence = False
    for line in body.splitlines(keepends=True):
        if _FENCE_RE.match(line):
            in_fence = not in_fence
            out_lines.append(line)
            continue
        if in_fence:
            out_lines.append(line)
            continue

        # Split on inline backtick spans, transforming only the non-code parts.
        parts = re.split(r"(`+[^`\n]*`+)", line)
        for i, part in enumerate(parts):
            if i % 2 == 0:
                parts[i] = _expand_segment(part, owner, repo_name)
        out_lines.append("".join(parts))

    return "".join(out_lines)
