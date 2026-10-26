"""Dumps the Python app's binding registry to bindings.json, for the Rust keymap's registry golden test.

Run from the repository root with an empty HOME, so `notifications.open` gets the default config:

    HOME=$(mktemp -d) uv run python crates/lazy-github-tui/tests/fixtures/generate_bindings.py

`--out DIR` writes it to DIR instead of this directory; the `bindings_goldens_are_fresh` test uses it.
"""

import argparse
import json
from pathlib import Path

from lazy_github.lib.bindings import LazyGithubBindings


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path(__file__).parent, help="directory to write bindings.json to")
    args = parser.parse_args()

    bindings = [
        {
            "id": binding.id,
            "key": binding.key,
            "action": binding.action,
            "description": binding.description,
            "show": binding.show,
            "key_display": binding.key_display,
            "tooltip": binding.tooltip,
        }
        for binding in LazyGithubBindings.all()
    ]
    out = args.out / "bindings.json"
    out.write_text(json.dumps(bindings, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
