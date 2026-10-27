"""Dumps what Textual's `Input` does with each of its keys, for the Rust text input's golden test: input_keys.json.

A headless Textual app with one focused `Input` plays each sequence below: it sets the value and the cursor, then
presses the keys one at a time and records `value`, `cursor_position` and `selection` (start, end) after each. The
in-app clipboard is empty, so `ctrl+v` pastes nothing over the selection. `crates/lazy-github-tui/tests/input_keys.rs`
replays the sequences on `TextInput`.

Run from the repository root:

    uv run python crates/lazy-github-tui/tests/fixtures/generate_input_keys.py

`--out DIR` writes it to DIR instead of this directory; the `input_keys_golden_is_fresh` test uses it.
"""

import argparse
import asyncio
import json
from pathlib import Path
from typing import Any

from textual.app import App, ComposeResult
from textual.widgets import Input

TEXT = "foo-bar baz"
UNICODE = "héllo wörld"

# (name, value, cursor, keys). Every key `INPUT_BINDINGS` binds is pressed at least once, with and without a selection.
SEQUENCES: list[tuple[str, str, int, list[str]]] = [
    ("left-right", TEXT, 5, ["left", "left", "right", "home", "left", "end", "right"]),
    ("home-end-aliases", TEXT, 5, ["ctrl+a", "ctrl+e"]),
    ("word-moves", TEXT, 0, ["ctrl+right", "ctrl+right", "ctrl+right", "ctrl+right", "ctrl+left", "ctrl+left"]),
    ("word-moves-inside", TEXT, 6, ["ctrl+left", "ctrl+left", "ctrl+left"]),
    ("shift-left", TEXT, 2, ["shift+left", "shift+left", "shift+left", "right"]),
    ("shift-right", TEXT, 9, ["shift+right", "shift+right", "shift+right", "left"]),
    ("shift-across-anchor", TEXT, 5, ["shift+left", "shift+right", "shift+right", "shift+right"]),
    ("word-select-left", TEXT, 11, ["ctrl+shift+left", "ctrl+shift+left", "ctrl+shift+left", "ctrl+shift+left"]),
    ("word-select-right", TEXT, 0, ["ctrl+shift+right", "ctrl+shift+right", "ctrl+shift+right", "x"]),
    ("word-select-both-ways", TEXT, 6, ["ctrl+shift+left", "ctrl+shift+right", "ctrl+shift+right"]),
    ("shift-home-reanchors", TEXT, 5, ["shift+left", "shift+home", "shift+end", "left"]),
    ("shift-end-reanchors", TEXT, 5, ["shift+right", "shift+end", "shift+home", "right"]),
    ("select-all", TEXT, 5, ["ctrl+shift+a", "shift+left", "ctrl+shift+a", "home"]),
    ("select-all-end", TEXT, 5, ["ctrl+shift+a", "end"]),
    ("select-all-word-left", TEXT, 5, ["ctrl+shift+a", "ctrl+left"]),
    ("select-then-word-right", TEXT, 2, ["shift+left", "ctrl+right"]),
    ("select-then-type", TEXT, 4, ["shift+right", "shift+right", "shift+right", "x", "y"]),
    ("delete-left", TEXT, 4, ["backspace", "backspace", "shift+right", "backspace", "home", "backspace"]),
    ("delete-right", TEXT, 3, ["delete", "ctrl+d", "shift+left", "delete", "end", "delete"]),
    ("delete-left-word", TEXT, 11, ["ctrl+w", "ctrl+w", "shift+left", "ctrl+w"]),
    ("delete-right-word", TEXT, 0, ["ctrl+f", "shift+right", "ctrl+f", "ctrl+f"]),
    ("delete-all", TEXT, 4, ["ctrl+u", "shift+right", "ctrl+u", "ctrl+k"]),
    ("delete-all-right-with-selection", TEXT, 4, ["shift+left", "ctrl+k"]),
    ("delete-all-left-with-selection", TEXT, 4, ["shift+end", "ctrl+u"]),
    ("paste-empty-clipboard", TEXT, 4, ["ctrl+v", "shift+right", "ctrl+v"]),
    ("enter-keeps-selection", TEXT, 4, ["shift+left", "enter"]),
    ("unicode", UNICODE, 11, ["ctrl+shift+left", "shift+left", "x", "shift+home", "shift+right", "y", "ctrl+left"]),
    ("empty", "", 0, ["shift+left", "shift+right", "ctrl+shift+a", "shift+home", "shift+end", "backspace"]),
]


class InputApp(App[None]):
    def compose(self) -> ComposeResult:
        yield Input()


def state(input: Input) -> dict[str, Any]:
    return {"value": input.value, "cursor": input.cursor_position, "selection": list(input.selection)}


async def play() -> list[dict[str, Any]]:
    app = InputApp()
    sequences = []
    async with app.run_test() as pilot:
        input = app.query_one(Input)
        input.focus()
        await pilot.pause()
        for name, value, cursor, keys in SEQUENCES:
            input.value = value
            input.cursor_position = cursor
            await pilot.pause()
            steps = []
            for key in keys:
                await pilot.press(key)
                steps.append({"key": key} | state(input))
            sequences.append({"name": name, "value": value, "cursor": cursor, "steps": steps})
    return sequences


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path(__file__).parent, help="directory to write the golden to")
    args = parser.parse_args()
    sequences = asyncio.run(play())
    (args.out / "input_keys.json").write_text(json.dumps(sequences, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
