"""Dumps the Python app's binding registry to bindings.json, for the Rust keymap's registry golden test.

Run from the repository root with an empty HOME, so `notifications.open` gets the default config:

    HOME=$(mktemp -d) uv run python crates/lazy-github-tui/tests/fixtures/generate_bindings.py
"""

import json
from pathlib import Path

from lazy_github.lib.bindings import LazyGithubBindings


def main() -> None:
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
    out = Path(__file__).with_name("bindings.json")
    out.write_text(json.dumps(bindings, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
