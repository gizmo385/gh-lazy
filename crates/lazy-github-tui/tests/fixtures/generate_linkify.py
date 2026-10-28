"""Dumps how markdown-it-py's `gfm-like` preset, which Textual's `Markdown` uses, links and normalizes the markdown in
linkify_corpus.toml, for the Rust markdown parser's golden test: linkify.json.

Each case is parsed with `MarkdownIt("gfm-like")`. For each inline token (a paragraph's or heading's text) it records
the children Textual draws: their type, `content`, a link's `href` and an image's `src`, and an image's children's
contents (Textual adds them as the image's text). `crates/lazy-github-tui/tests/linkify.rs` lays each case out with the
Rust renderer and compares the text, emphasis and link targets.

Run from the repository root:

    uv run python crates/lazy-github-tui/tests/fixtures/generate_linkify.py

`--out DIR` writes it to DIR instead of this directory; the `linkify_golden_is_fresh` test uses it.
"""

import argparse
import json
import tomllib
from pathlib import Path
from typing import Any

from markdown_it import MarkdownIt
from markdown_it.token import Token

CORPUS = Path(__file__).parent / "linkify_corpus.toml"


def child(token: Token) -> dict[str, Any]:
    out: dict[str, Any] = {"type": token.type, "content": token.content}
    if token.type == "link_open":
        out["href"] = token.attrs.get("href", "")
    if token.type == "image":
        out["src"] = token.attrs.get("src", "")
        out["children"] = [grandchild.content for grandchild in token.children or []]
    return out


def dump(md: MarkdownIt, source: str) -> dict[str, Any]:
    blocks = [[child(c) for c in token.children or []] for token in md.parse(source) if token.type == "inline"]
    return {"source": source, "blocks": blocks}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path(__file__).parent, help="directory to write the golden to")
    args = parser.parse_args()
    md = MarkdownIt("gfm-like")
    cases = tomllib.loads(CORPUS.read_text())["cases"]
    golden = [dump(md, source) for source in cases]
    (args.out / "linkify.json").write_text(json.dumps(golden, indent=1, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
