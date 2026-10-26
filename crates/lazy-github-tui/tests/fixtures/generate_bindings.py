"""Dumps the Python app's bindings for the Rust keymap's golden tests: bindings.json, contexts.json and keys.json.

- bindings.json: the registry, `LazyGithubBindings.all()`.
- contexts.json: each class's merged `BINDINGS`, in Textual's order: every widget or screen class under
  `lazy_github/ui/` that defines `BINDINGS` itself, and the Textual widgets the app uses directly.
- keys.json: the key names and footer display Textual gives every default key string, and each override spelling
  applied to `app.open_help` the way `App.set_keymap` applies it.

Run from the repository root with an empty HOME, so `notifications.open` gets the default config:

    HOME=$(mktemp -d) uv run python crates/lazy-github-tui/tests/fixtures/generate_bindings.py

`--out DIR` writes them to DIR instead of this directory; the `bindings_goldens_are_fresh` test uses it.
"""

import argparse
import importlib
import inspect
import json
from pathlib import Path
from typing import Any

from textual.app import App
from textual.binding import Binding, BindingsMap
from textual.containers import VerticalScroll
from textual.dom import DOMNode
from textual.widgets import Input, Log, Tabs

import lazy_github.lib.bindings as bindings_module
from lazy_github.lib.bindings import LazyGithubBindings

# The Textual classes the app uses as they are, under the name it imports them by.
TEXTUAL_CLASSES: dict[str, type[DOMNode]] = {
    "textual.containers.VerticalScroll": VerticalScroll,
    "textual.widgets.Log": Log,
    "textual.widgets.Tabs": Tabs,
    "textual.widgets.Input": Input,
}

# Override spellings that the parity matrix, keymap-overrides.toml and unknown-fields.json use, and the edge cases
# `Keymap::new` documents.
OVERRIDES = [
    "?",
    "question_mark",
    "/",
    "slash",
    "$",
    "dollar_sign",
    "^",
    "circumflex_accent",
    "ctrl+t",
    "x, ctrl+t",
    "x,ctrl+t",
    "q, escape",
    "9",
    "M",
    "bogus",
    "Enter",
    "ctrl+",
    "shift+1",
    "comma",
    "shift+enter",
    "ctrl+m",
    "",
    " , ",
]


def binding_fields(binding: Binding) -> dict[str, Any]:
    return {
        "action": binding.action,
        "description": binding.description,
        "show": binding.show,
        "priority": binding.priority,
        "system": binding.system,
        "key_display": binding.key_display,
        "tooltip": binding.tooltip,
    }


def ui_classes() -> dict[str, type[DOMNode]]:
    """Every DOMNode subclass defined under lazy_github/ui/ with BINDINGS in its own body.

    `lazy_github/ui` has no `__init__.py`, so `pkgutil.walk_packages` would find only `app.py`: walk the files.
    """
    package_root = Path(bindings_module.__file__).resolve().parents[1]
    classes = {}
    for path in sorted((package_root / "ui").rglob("*.py")):
        module_name = ".".join(path.relative_to(package_root.parent).with_suffix("").parts)
        module = importlib.import_module(module_name)
        for cls in vars(module).values():
            if (
                inspect.isclass(cls)
                and issubclass(cls, DOMNode)
                and cls.__module__ == module_name
                and "BINDINGS" in cls.__dict__
            ):
                classes[f"{module_name}.{cls.__qualname__}"] = cls
    return classes


def contexts() -> list[dict[str, Any]]:
    classes = ui_classes() | TEXTUAL_CLASSES
    return [
        {
            "class": name,
            "bindings": [
                {"key": key, "id": binding.id} | binding_fields(binding)
                for key, binding in classes[name]._merge_bindings()
            ],
        }
        for name in sorted(classes)
    ]


def probe(app: App, bindings: BindingsMap) -> dict[str, list[str]]:
    """The keys of a map that holds one binding, and the footer display of each."""
    return {
        "keys": list(bindings.key_to_bindings),
        "display": [app.get_key_display(found[0]) for found in bindings.key_to_bindings.values()],
    }


def keys(registry: list[dict[str, Any]], context_list: list[dict[str, Any]]) -> dict[str, Any]:
    app = App()
    raw_keys = {binding["key"] for binding in registry}
    raw_keys |= {binding["key"] for context in context_list for binding in context["bindings"]}
    defaults = [
        {"raw": raw} | probe(app, BindingsMap([Binding(raw, "probe", "Probe")])) for raw in sorted(raw_keys)
    ]
    overrides = []
    for raw in OVERRIDES:
        keymap = App._normalize_keymap({"app.open_help": raw})
        bindings = BindingsMap([LazyGithubBindings.OPEN_HELP])
        bindings.apply_keymap(keymap)
        overrides.append({"raw": raw, "normalized": keymap["app.open_help"]} | probe(app, bindings))
    return {"defaults": defaults, "overrides": overrides}


def write(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--out", type=Path, default=Path(__file__).parent, help="directory to write the goldens to")
    args = parser.parse_args()

    registry = [
        {"id": binding.id, "key": binding.key} | binding_fields(binding) for binding in LazyGithubBindings.all()
    ]
    context_list = contexts()
    write(args.out / "bindings.json", registry)
    write(args.out / "contexts.json", context_list)
    write(args.out / "keys.json", keys(registry, context_list))


if __name__ == "__main__":
    main()
