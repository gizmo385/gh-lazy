"""Writes cases.json: Python's str() of row tuples, the text LazyGithub's table search matches against.

The Rust table search (`components/table/py_repr.rs`) must reproduce these byte for byte. Regenerate with:

    uv run python crates/lazy-github-tui/tests/fixtures/py_repr/generate.py
"""

import json
import sys
from datetime import datetime
from pathlib import Path

CASES: list[tuple[str, tuple]] = [
    ("ints", (0, 42, -7, 9007199254740993)),
    ("one_int", (5,)),
    ("ascii", ("parity-bot", "gh-lazy")),
    ("single_quote", ("it's",)),
    ("double_quote", ('say "hi"',)),
    ("both_quotes", ("it's \"both\"",)),
    ("backslash", ("C:\\path\\to",)),
    ("newline_tab_cr", ("line\nnext\tcol\rback",)),
    ("del", ("a\x7fb",)),
    ("c0_and_c1", ("\x00\x1b\x85\xa0",)),
    ("zero_width_space", ("zero\u200bwidth",)),
    ("emoji", ("ship it 🚀",)),
    ("cjk", ("修复中文标题",)),
    ("favorite_markup", ("[green]★[/]", "parity-bot", "lazy-github", "✘")),
    ("not_favorite", ("☆", "parity-org", "dotfiles", "")),
    ("line_and_paragraph_separators", ("a\u2028b\u2029c",)),
    ("private_use_and_unassigned", ("\ue000\u0378\U000e0001\U0010ffff",)),
    ("ideographic_space", ("a\u3000b",)),
    ("datetime_minutes", (datetime(2026, 1, 5, 3, 4),)),
    ("datetime_seconds", (datetime(2026, 10, 25, 23, 59, 7),)),
    ("datetime_microseconds", (datetime(2026, 10, 25, 23, 59, 7, 120),)),
    ("datetime_zero_seconds_with_micros", (datetime(2026, 2, 3, 0, 0, 0, 5),)),
    ("mixed_row", (1234, "Fix the [bug] here", datetime(2025, 12, 31, 12, 0, 30), "it's")),
    ("empty", ()),
]


def cell(value: object) -> dict:
    if isinstance(value, datetime):
        assert value.tzinfo is None
        return {
            "datetime": [
                value.year,
                value.month,
                value.day,
                value.hour,
                value.minute,
                value.second,
                value.microsecond,
            ]
        }
    if isinstance(value, int):
        return {"int": value}
    assert isinstance(value, str)
    return {"str": value}


def main() -> None:
    cases = [{"name": name, "cells": [cell(v) for v in row], "repr": str(row)} for name, row in CASES]
    out = Path(__file__).with_name("cases.json")
    out.write_text(json.dumps(cases, indent=2, ensure_ascii=True) + "\n")
    print(f"wrote {len(cases)} cases to {out} with Python {sys.version.split()[0]}")


if __name__ == "__main__":
    main()
