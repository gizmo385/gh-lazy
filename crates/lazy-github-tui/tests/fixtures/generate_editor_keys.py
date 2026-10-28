"""Dumps what Textual's `TextArea` does with its movement and editing keys, for the Rust text editor's golden test:
editor_keys.json.

A headless Textual app holds the two editors LazyGithub uses: `TextArea.code_editor(soft_wrap=True,
tab_behavior="focus")` sized so its text wraps at 40 columns (with a one-digit gutter and no scrollbar), and a plain
`TextArea(soft_wrap=True, tab_behavior="focus")` that wraps at 20. Each case loads its text, moves the cursor to its
start, then presses the keys one at a time (or pastes) and records the text, the selection (start, end) and the
vertical scroll after each. `crates/lazy-github-tui/tests/editor_keys.rs` replays the cases on `TextEditor` drawn at
the same size. Every case starts with an empty in-app clipboard, unless it sets one, and the clipboard is recorded
after each step too.

Run from the repository root:

    uv run python crates/lazy-github-tui/tests/fixtures/generate_editor_keys.py

`--out DIR` writes it to DIR instead of this directory; the `editor_keys_golden_is_fresh` test uses it.
"""

import argparse
import asyncio
import json
from pathlib import Path
from typing import Any

from textual import events
from textual.app import App, ComposeResult
from textual.widgets import TextArea

# Outer sizes: border 2 + padding 2 (+ the code editor's gutter 3 and the cursor's column 1) around 40 / 20 columns.
SIZES = {"code": (48, 8), "plain": (25, 8)}

PROSE = "The quick brown fox jumps over the lazy dog and keeps running far away from home"
MULTI = "first line\n  indented line\n\nlast"
CJK = "中文字符测试中文字符测试中文字符测试中文字符测试中文"
ACCENT = "héllo wörld\nnaïve café"
LONG = "a" * 50 + " end"
TABS = "a\tb\tc d\te"
NUMBERED = "\n".join(f"line {i}" for i in range(1, 13))
WORDS = "foo.bar(baz)  qux\n  spaced   out  \nend"

# (name, variant, text, (row, column), steps): a step is a key in Textual's syntax, or {"paste": text}. The cases
# start with an empty clipboard.
CASES: list[tuple[str, str, str, tuple[int, int], list[Any]]] = [
    # Up and down over wrapped rows, keeping the column from the last sideways move.
    ("up-down-wrapped", "code", PROSE, (1, 0), ["down", "up", "up", "down", "down", "down", "up"]),
    ("up-down-from-end", "code", PROSE, (0, 79), ["up", "up", "up", "down", "down"]),
    ("up-down-lines", "code", MULTI, (1, 12), ["up", "down", "down", "down", "up", "up", "up"]),
    ("up-down-remember-column", "code", MULTI, (0, 9), ["down", "down", "down", "up", "left", "up"]),
    ("up-down-plain", "plain", PROSE, (0, 30), ["up", "down", "down", "down", "down", "up", "up", "up"]),
    ("up-down-cjk", "code", CJK, (0, 22), ["up", "down", "down", "up"]),
    ("up-down-cjk-plain", "plain", CJK, (0, 3), ["down", "down", "up", "right", "down"]),
    # Left and right across line ends.
    ("left-right", "code", MULTI, (1, 0), ["left", "left", "right", "right", "right"]),
    ("left-at-start", "code", MULTI, (0, 0), ["left", "right"]),
    ("right-at-end", "code", MULTI, (3, 3), ["right", "right", "left"]),
    ("left-right-wide", "code", ACCENT, (0, 10), ["right", "right", "left", "left"]),
    # Word moves.
    ("word-right", "code", WORDS, (0, 0), ["ctrl+right"] * 9),
    ("word-left", "code", WORDS, (2, 3), ["ctrl+left"] * 9),
    ("word-moves-accent", "code", ACCENT, (0, 0), ["ctrl+right", "ctrl+right", "ctrl+right", "ctrl+left"]),
    ("word-moves-cjk", "plain", CJK, (0, 5), ["ctrl+right", "ctrl+left", "ctrl+left"]),
    # Home and end, smart home, wrapped rows.
    ("smart-home", "code", MULTI, (1, 8), ["home", "home", "home", "right", "home", "ctrl+a", "ctrl+a"]),
    ("smart-home-before-text", "code", MULTI, (1, 1), ["home", "end", "ctrl+e", "home"]),
    ("home-end-wrapped", "code", PROSE, (0, 50), ["home", "end", "end", "home", "down", "end", "home"]),
    ("home-end-wrapped-plain", "plain", PROSE, (0, 25), ["home", "end", "up", "end", "ctrl+a", "ctrl+e"]),
    ("home-end-long-word", "plain", LONG, (0, 30), ["home", "end", "down", "end", "home", "up"]),
    ("home-end-tabs", "plain", TABS, (0, 5), ["home", "end", "left", "up", "down"]),
    # Pages.
    ("page-down-up", "code", NUMBERED, (0, 3), ["pagedown", "pagedown", "pagedown", "pageup", "pageup", "pageup"]),
    ("page-down-wrapped", "plain", PROSE + "\n" + NUMBERED, (0, 4), ["pagedown", "pagedown", "pageup", "down"]),
    ("down-scrolls", "code", NUMBERED, (0, 0), ["down"] * 7 + ["up"] * 7),
    # Typing, enter and deleting characters.
    ("type", "code", MULTI, (0, 5), ["x", "space", "é", "中", "enter", "q", "enter", "enter"]),
    ("type-wraps", "plain", "abcdefghijklmnopqrs", (0, 19), ["t", "u", "space", "v", "backspace", "backspace"]),
    ("backspace-joins", "code", MULTI, (1, 0), ["backspace", "backspace", "end", "backspace"]),
    ("backspace-at-start", "code", MULTI, (0, 0), ["backspace"]),
    ("delete-joins", "code", MULTI, (0, 10), ["delete", "ctrl+d", "end", "delete", "delete"]),
    ("delete-at-end", "code", MULTI, (3, 4), ["delete", "ctrl+d"]),
    ("delete-wide", "plain", CJK, (0, 1), ["delete", "backspace", "right", "delete"]),
    # Deleting words.
    ("delete-word-left", "code", WORDS, (0, 17), ["ctrl+w"] * 6),
    ("delete-word-left-lines", "code", WORDS, (1, 2), ["ctrl+w", "ctrl+w", "ctrl+w"]),
    ("delete-word-left-at-start", "code", WORDS, (0, 0), ["ctrl+w"]),
    ("delete-word-right", "code", WORDS, (0, 0), ["ctrl+f"] * 7),
    ("delete-word-right-spaces", "code", WORDS, (1, 0), ["ctrl+f", "ctrl+f", "ctrl+f", "ctrl+f", "ctrl+f"]),
    ("delete-word-right-at-end", "code", WORDS, (2, 3), ["ctrl+f"]),
    ("delete-word-right-line-end", "code", WORDS, (0, 17), ["ctrl+f", "ctrl+f"]),
    # Deleting to the line's start or end, and lines.
    ("delete-to-start", "code", MULTI, (1, 9), ["ctrl+u", "ctrl+u", "down", "ctrl+u"]),
    ("delete-to-start-wrapped", "plain", PROSE, (0, 30), ["ctrl+u", "ctrl+u"]),
    ("delete-to-end", "code", MULTI, (1, 4), ["ctrl+k", "ctrl+k", "ctrl+k", "ctrl+k"]),
    ("delete-to-end-empty-line", "code", MULTI, (2, 0), ["ctrl+k", "ctrl+k"]),
    ("delete-to-end-wrapped", "plain", PROSE, (0, 25), ["ctrl+k", "ctrl+k", "ctrl+k"]),
    ("delete-line", "code", MULTI, (1, 6), ["ctrl+shift+k", "ctrl+shift+k", "ctrl+shift+k", "ctrl+shift+k"]),
    ("delete-line-keeps-column", "code", NUMBERED, (4, 5), ["ctrl+shift+k", "up", "ctrl+shift+k"]),
    ("delete-line-last", "code", MULTI, (3, 2), ["ctrl+shift+k", "ctrl+shift+k"]),
    # Paste: every line goes in.
    ("paste-lines", "code", MULTI, (0, 5), [{"paste": "a\r\nb\nc"}, "x", {"paste": "end\n"}]),
    ("paste-wrapping", "plain", "", (0, 0), [{"paste": PROSE}, "up", {"paste": "\n"}]),
    ("paste-gutter-grows", "code", MULTI, (3, 4), [{"paste": "\n".join("p" * i for i in range(1, 9))}, "up"]),
    # Edge cases: whitespace-only lines, tabs over rows, lone carriage returns, folded words.
    ("word-moves-blank-lines", "code", "a  \n   \n\n  b", (3, 3), ["ctrl+left"] * 5 + ["ctrl+right"] * 5),
    ("delete-word-left-blank", "code", "ab  \n   \ncd", (2, 0), ["ctrl+w", "ctrl+w", "ctrl+w", "ctrl+w"]),
    ("up-down-tabs", "plain", "\tab\tcd\tef\tgh\tij\n\t\tx", (0, 6), ["down", "down", "up", "up", "end", "down"]),
    ("paste-carriage-returns", "code", "xy", (0, 1), [{"paste": "1\r2\r"}, {"paste": "\u2028z"}]),
    ("fold-delete", "plain", LONG, (0, 25), ["ctrl+w", "ctrl+f", "ctrl+u", "ctrl+k", "ctrl+k"]),
    ("type-at-wrap", "code", "x" * 39 + " yy", (0, 39), ["z", "space", "left", "backspace", "down", "up"]),
    ("page-plain-long", "plain", PROSE + " " + PROSE + "\n" + PROSE, (0, 0), ["pagedown"] * 3 + ["pageup"] * 3),
    ("delete-line-wrapped", "plain", PROSE + "\nnext", (0, 45), ["ctrl+shift+k", "ctrl+shift+k"]),
    ("delete-to-end-wrapped-middle", "plain", PROSE, (0, 20), ["ctrl+k", "end", "ctrl+k", "home", "ctrl+u"]),
    # Selection keys (#257).
    ("select-shift-arrows", "code", MULTI, (1, 4), ["shift+right", "shift+right", "shift+down", "shift+left", "shift+up", "shift+up", "right"]),
    ("select-shift-left-collapse", "code", MULTI, (1, 4), ["shift+left", "shift+left", "left", "shift+right", "shift+right", "right"]),
    ("select-wrapped-rows", "plain", PROSE, (0, 25), ["shift+down", "shift+down", "shift+up", "shift+up", "shift+up", "down"]),
    ("select-words", "code", WORDS, (0, 4), ["ctrl+shift+right", "ctrl+shift+right", "ctrl+shift+left", "ctrl+shift+left", "ctrl+shift+left", "ctrl+shift+left", "end"]),
    ("select-word-ends", "code", WORDS, (2, 3), ["ctrl+shift+right", "home", "ctrl+shift+left"]),
    ("select-home-end", "code", MULTI, (1, 8), ["shift+home", "shift+home", "shift+end", "shift+end", "shift+home"]),
    ("select-home-end-wrapped", "plain", PROSE, (0, 30), ["shift+end", "shift+home", "shift+home", "up"]),
    ("select-line", "plain", PROSE + "\nnext", (0, 30), ["f6", "x", "f6", "f6", "backspace"]),
    ("select-line-empty", "code", MULTI, (2, 0), ["f6", "down", "f6", "up"]),
    ("select-all", "code", MULTI, (1, 3), ["f7", "f7", "left", "f7", "right", "f7", "enter"]),
    ("select-then-type", "code", MULTI, (0, 2), ["shift+down", "q", "shift+left", "shift+left", "enter"]),
    ("select-then-delete", "code", MULTI, (0, 2), ["shift+down", "delete", "shift+end", "backspace", "shift+home", "ctrl+w"]),
    ("select-then-delete-word", "code", WORDS, (0, 4), ["ctrl+shift+right", "ctrl+f", "shift+left", "ctrl+f"]),
    ("select-then-delete-to-ends", "code", MULTI, (1, 6), ["shift+left", "ctrl+u", "shift+right", "ctrl+k", "shift+left", "ctrl+k"]),
    ("select-then-delete-lines", "code", NUMBERED, (2, 3), ["shift+down", "shift+down", "ctrl+shift+k", "shift+down", "home", "ctrl+shift+k"]),
    ("select-lines-ending-at-start", "code", NUMBERED, (2, 3), ["shift+down", "shift+home", "ctrl+shift+k"]),
    ("select-up-from-first-row", "code", MULTI, (0, 3), ["shift+up", "shift+down", "shift+down", "shift+down", "shift+down"]),
    # Clipboard keys (#257).
    ("cut-selection", "code", MULTI, (1, 2), ["shift+end", "ctrl+x", "end", "ctrl+v", "ctrl+v"]),
    ("cut-lines", "code", MULTI, (1, 5), ["ctrl+x", "ctrl+x", "ctrl+v", "up", "ctrl+v"]),
    ("cut-last-line", "code", MULTI, (3, 2), ["ctrl+x", "ctrl+v", "ctrl+x"]),
    ("cut-only-line", "plain", "lonely", (0, 3), ["ctrl+x", "ctrl+v", "ctrl+v"]),
    ("copy-selection", "code", MULTI, (0, 0), ["shift+down", "shift+right", "ctrl+c", "end", "ctrl+v", "super+c", "ctrl+c"]),
    ("copy-without-selection", "code", MULTI, (1, 2), ["ctrl+c", "super+c", "ctrl+v"]),
    ("copy-across-lines-and-paste-over", "plain", PROSE + "\nnext", (0, 70), ["shift+down", "ctrl+c", "f7", "ctrl+v"]),
    # Undo and redo (#257).
    ("undo-redo", "code", "", (0, 0), ["h", "i", "space", "y", "o", "enter", "a", "b", {"paste": "pasted"}, "ctrl+z", "ctrl+z", "ctrl+z", "ctrl+y", "ctrl+y", "ctrl+y", "ctrl+y"]),
    ("undo-moves-split-batches", "code", MULTI, (0, 5), ["x", "y", "right", "z", "w", "left", "ctrl+z", "ctrl+z", "ctrl+z", "ctrl+z"]),
    ("undo-replace-switch", "code", MULTI, (0, 0), ["a", "b", "backspace", "backspace", "c", "ctrl+z", "ctrl+z", "ctrl+z", "ctrl+y"]),
    ("undo-selection-restored", "code", MULTI, (1, 2), ["shift+right", "shift+right", "shift+right", "q", "ctrl+z", "ctrl+y", "ctrl+z"]),
    ("undo-select-all-typing", "plain", PROSE, (0, 4), ["f7", "x", "y", "ctrl+z", "ctrl+z", "ctrl+y"]),
    ("redo-cleared-by-edit", "code", MULTI, (3, 4), ["!", "?", "enter", "ctrl+z", "x", "ctrl+y", "ctrl+z", "ctrl+z"]),
    ("undo-deletes", "code", WORDS, (0, 17), ["ctrl+w", "ctrl+w", "backspace", "ctrl+shift+k", "ctrl+z", "ctrl+z", "ctrl+z", "ctrl+y"]),
    ("undo-cut-and-paste", "code", MULTI, (1, 3), ["ctrl+x", "ctrl+v", "ctrl+z", "ctrl+z", "ctrl+y", "ctrl+y"]),
    ("undo-nothing", "code", MULTI, (1, 3), ["ctrl+z", "ctrl+y"]),
    ("undo-long-typing", "plain", "", (0, 0), ["a"] * 105 + ["ctrl+z", "ctrl+z", "ctrl+z"]),
    # The empty editor.
    (
        "empty",
        "code",
        "",
        (0, 0),
        ["up", "down", "left", "right", "home", "end", "ctrl+left", "ctrl+right", "backspace", "delete"]
        + ["ctrl+w", "ctrl+f", "ctrl+u", "ctrl+k", "ctrl+shift+k", "pageup", "pagedown", "a"],
    ),
]

# (name, variant, text, (row, column), clipboard, steps): pasting a clipboard copied elsewhere.
CLIPBOARD_CASES: list[tuple[str, str, str, tuple[int, int], str, list[Any]]] = [
    ("paste-clipboard-lines", "code", MULTI, (1, 2), "one\ntwo\n", ["ctrl+v", "ctrl+v", "shift+up", "ctrl+v", "ctrl+z"]),
    ("paste-clipboard-over-selection", "plain", PROSE, (0, 4), "fast", ["ctrl+shift+right", "ctrl+v", "ctrl+z", "ctrl+z"]),
]


class EditorApp(App[None]):
    CSS = "\n".join(f"#{variant} {{ width: {w}; height: {h}; }}" for variant, (w, h) in SIZES.items())

    def compose(self) -> ComposeResult:
        yield TextArea.code_editor(soft_wrap=True, tab_behavior="focus", id="code")
        yield TextArea(soft_wrap=True, tab_behavior="focus", id="plain")


def state(app: App[None], editor: TextArea) -> dict[str, Any]:
    start, end = editor.selection
    return {
        "text": editor.text,
        "selection": [list(start), list(end)],
        "scroll": int(editor.scroll_offset.y),
        "clipboard": app.clipboard,
    }


async def play() -> list[dict[str, Any]]:
    app = EditorApp()
    cases = []
    async with app.run_test(size=(80, 24)) as pilot:
        plain = [(name, variant, text, cursor, "", steps) for name, variant, text, cursor, steps in CASES]
        for name, variant, text, cursor, clipboard, steps in plain + CLIPBOARD_CASES:
            editor = app.query_one(f"#{variant}", TextArea)
            editor.focus()
            editor.text = text
            await pilot.pause()
            editor.scroll_to(0, 0, animate=False)
            editor.move_cursor(cursor)
            app.copy_to_clipboard(clipboard)
            await pilot.pause()
            recorded = []
            for step in steps:
                if isinstance(step, dict):
                    # As a terminal's paste arrives: at the app, which forwards it to the focused widget.
                    app.post_message(events.Paste(step["paste"]))
                else:
                    await pilot.press(step)
                await pilot.pause()
                recorded.append((step if isinstance(step, dict) else {"key": step}) | state(app, editor))
            cases.append(
                {
                    "name": name,
                    "variant": variant,
                    "size": list(SIZES[variant]),
                    "text": text,
                    "cursor": list(cursor),
                    "clipboard": clipboard,
                    "steps": recorded,
                }
            )
    return cases


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path(__file__).parent, help="directory to write the golden to")
    args = parser.parse_args()
    cases = asyncio.run(play())
    (args.out / "editor_keys.json").write_text(json.dumps(cases, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
