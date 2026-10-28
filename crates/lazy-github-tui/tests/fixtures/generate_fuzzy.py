"""Dumps how Textual's fuzzy matcher, which the command palette and the settings search use, scores and highlights each
query in QUERIES against each name in NAMES, for the Rust matcher's golden test: fuzzy.json.

For each pair it records `FuzzySearch().match(query, name)`'s score and offsets (score 0 and no offsets for no match),
and the offsets `Matcher(query).highlight(name)` styles. `crates/lazy-github-tui/tests/fuzzy.rs` runs the Rust matcher
on every pair and compares them.

Run from the repository root:

    uv run python crates/lazy-github-tui/tests/fixtures/generate_fuzzy.py

`--out DIR` writes it to DIR instead of this directory; the `fuzzy_goldens_are_fresh` test uses it.
"""

import argparse
import json
from pathlib import Path
from typing import Any

from textual.fuzzy import FuzzySearch, Matcher
from textual.style import Style

NAMES = [
    # LazyGithub's palette commands (`MainScreenCommandProvider.commands`).
    "Toggle Command Log",
    "Toggle Workflows",
    "Toggle Issues",
    "Toggle Pull Requests",
    "Change Settings",
    "Debug Info",
    "Open Bug Report",
    "Refresh notifications",
    # Textual's system commands (`App.get_system_commands`).
    "Theme",
    "Quit",
    "Keys",
    "Minimize",
    "Maximize",
    "Screenshot",
    # Settings field names (the settings search).
    "show_command_log",
    "preferred_merge_method",
    # Spaces, digits, `_` and repeated letters.
    "Tab 22 bb b",
    "mississippi",
    "v2_build 10",
]

QUERIES = [
    # Single letters.
    "t",
    "q",
    "i",
    "m",
    "s",
    "_",
    "2",
    " ",
    # Substrings.
    "tog",
    "keys",
    "log",
    "e i",
    "ss",
    "command_log",
    # Exact names.
    "quit",
    "theme",
    "toggle issues",
    "mississippi",
    # Subsequences.
    "tgi",
    "tpr",
    "cmlg",
    "ref",
    "pmm",
    "tcl",
    "sc",
    "oe",
    "mze",
    "dbg",
    "v21",
    # Hit Textual's `last_index` cut, which changes the winning offsets.
    "iipi",
    "bbb",
    "tb b",
    # Uppercase input.
    "TOG",
    "QuIt",
    "TPR",
    # No match.
    "zzz",
    "tiq",
    "quitx",
]


def highlighted(query: str, name: str) -> list[int]:
    style = Style(bold=True, underline=True)
    content = Matcher(query, match_style=style).highlight(name)
    return sorted({offset for span in content.spans for offset in range(span.start, span.end)})


def dump(search: FuzzySearch, query: str, name: str) -> dict[str, Any]:
    score, offsets = search.match(query, name)
    return {
        "query": query,
        "name": name,
        "score": score,
        "offsets": list(offsets),
        "highlights": highlighted(query, name),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path(__file__).parent, help="directory to write the golden to")
    args = parser.parse_args()
    search = FuzzySearch()
    golden = [dump(search, query, name) for query in QUERIES for name in NAMES]
    (args.out / "fuzzy.json").write_text(json.dumps(golden, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
