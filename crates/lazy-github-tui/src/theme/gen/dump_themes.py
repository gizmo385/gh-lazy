"""Dumps Textual's built-in theme variables as Rust source, for the Rust `lazy-github-tui` theme module.

Run from the repository root with:

    uv run --frozen python crates/lazy-github-tui/src/theme/gen/dump_themes.py > crates/lazy-github-tui/src/theme/palette.rs

It prints `palette.rs` to stdout and writes `crates/lazy-github-tui/tests/fixtures/themes/composites.json`: Textual's
own compositing results for the color pairs the Rust tests check. It imports only `textual` (and `rich`, which Textual
depends on), and fails loudly on any value it doesn't understand, so bumping Textual can't silently drop or misread a
variable.

With `--downgrade` it prints nothing and instead writes `crates/lazy-github-tui/tests/fixtures/themes/downgrade.json`:
for every opaque RGB variable of every theme, the xterm-256 and 16-color indices Rich's `Color.downgrade` picks.

    uv run --frozen python crates/lazy-github-tui/src/theme/gen/dump_themes.py --downgrade
"""

import json
import re
import sys
from importlib.metadata import version
from pathlib import Path

from rich.color import Color as RichColor
from rich.color import ColorSystem
from textual.color import ANSI_COLORS, Color
from textual.theme import BUILTIN_THEMES

SCRIPT = "crates/lazy-github-tui/src/theme/gen/dump_themes.py"
FIXTURES = Path(__file__).resolve().parents[3] / "tests" / "fixtures" / "themes"
COMPOSITES = FIXTURES / "composites.json"
DOWNGRADE = FIXTURES / "downgrade.json"
TEXTUAL_VERSION = version("textual")
RICH_VERSION = version("rich")

# Textual's text-style flags, as ratatui modifiers. Textual also has `overline` and `uu`, which ratatui can't draw; no
# built-in theme uses them, and an unknown flag fails below.
TEXT_STYLE_FLAGS = {
    "b": "BOLD",
    "bold": "BOLD",
    "d": "DIM",
    "dim": "DIM",
    "i": "ITALIC",
    "italic": "ITALIC",
    "u": "UNDERLINED",
    "underline": "UNDERLINED",
    "r": "REVERSED",
    "reverse": "REVERSED",
    "s": "CROSSED_OUT",
    "strike": "CROSSED_OUT",
    "blink": "SLOW_BLINK",
}

# The pairs in composites.json: (foreground, background), as Textual CSS.
COMPOSITE_PAIRS = [
    ("$text", "$background"),
    ("$text", "$surface"),
    ("$text", "$panel"),
    ("$text", "$panel-lighten-1"),
    ("$text-muted", "$surface"),
    ("$foreground-muted", "$surface"),
    ("$block-cursor-foreground", "$block-cursor-background"),
    ("$block-cursor-blurred-background", "$surface"),
    ("$boost", "$background"),
    ("$foreground 5%", "$surface"),
]

PERCENT = re.compile(r"(?P<base>.*\S)\s+(?P<pct>\d+(?:\.\d+)?)%")


def fail(message: str) -> None:
    sys.exit(f"dump_themes.py: {message}")


def is_text_style(name: str) -> bool:
    return "text-style" in name or "link-style" in name


def parse_color(value: str) -> Color:
    """Parses a color variable the way Textual's CSS parser does, keeping `auto`."""
    base, alpha = value, None
    if match := PERCENT.fullmatch(value):
        base, alpha = match["base"], float(match["pct"]) / 100.0
    if base == "initial":
        base = "transparent"
    if base == "auto":
        color = Color(255, 255, 255, auto=True)
    else:
        color = Color.parse(base)
    return color if alpha is None else color.multiply_alpha(alpha)


def ansi_variant(ansi: int) -> str:
    if ansi == -1:
        return "Default"
    return "".join(part.capitalize() for part in ANSI_COLORS[ansi].split("_"))


def rust_color(name: str, value: str) -> str:
    color = parse_color(value)
    if color.ansi is not None:
        return f"Ansi(AnsiColor::{ansi_variant(color.ansi)})"
    if color.auto:
        return f"Auto {{ alpha: {color.a!r} }}"
    if value in ("transparent", "initial"):
        return "Transparent"
    if not 0 <= color.a <= 1:
        fail(f"{name}: {value!r} has alpha {color.a}")
    if color.a == 1:
        return f"rgb(0x{color.r:02X}, 0x{color.g:02X}, 0x{color.b:02X})"
    return f"rgba(0x{color.r:02X}, 0x{color.g:02X}, 0x{color.b:02X}, {color.a!r})"


def rust_text_style(name: str, value: str) -> str:
    add, sub = [], []
    words = value.split()
    if words == ["none"]:
        words = []
    negate = False
    for word in words:
        if word == "not":
            negate = True
            continue
        flag = TEXT_STYLE_FLAGS.get(word)
        if flag is None:
            fail(f"{name}: unknown text-style flag {word!r} in {value!r}")
        (sub if negate else add).append(flag)
        negate = False
    if negate:
        fail(f"{name}: dangling `not` in {value!r}")

    def modifier(flags: list[str]) -> str:
        if not flags:
            return "Modifier::empty()"
        return ".union(".join(f"Modifier::{flag}" for flag in flags) + ")" * (len(flags) - 1)

    return f"TextStyle {{ add: {modifier(add)}, sub: {modifier(sub)} }}"


def rust_value(name: str, value: str) -> str:
    return rust_text_style(name, value) if is_text_style(name) else rust_color(name, value)


def field(name: str) -> str:
    return name.replace("-", "_")


def composite_result(variables: dict[str, str], fg: str, bg: str) -> str:
    """Textual's `Color.parse(bg) + Color.parse(fg)`, with `$variables` substituted."""

    def resolve(css: str) -> Color:
        return parse_color(re.sub(r"\$([a-z0-9-]+)", lambda m: variables[m[1]], css))

    under = resolve(bg)
    # `+` blends with `under`'s RGB and ignores its alpha, while the Rust test composites `under` over black first.
    # The two agree only for an opaque or fully transparent background, so refuse anything in between.
    if under.ansi is None and 0 < under.a < 1:
        fail(f"{fg} over {bg}: the background is semi-transparent ({under!r}); pick an opaque one")
    result = under + resolve(fg)
    if result.ansi is not None:
        return result.hex
    if result.a != 1:
        fail(f"{fg} over {bg} isn't opaque: {result!r}")
    return result.hex6


def opaque_rgb(value: str) -> Color | None:
    """The color of an opaque RGB variable, or None for ANSI, `auto`, transparent and translucent ones."""
    if value in ("transparent", "initial"):
        return None
    color = parse_color(value)
    if color.ansi is not None or color.auto or color.a != 1:
        return None
    return color


def write_downgrade(themes: list, shared: list[str]) -> None:
    """Writes downgrade.json: one line per variable, `"name": ["#RRGGBB", eight_bit, standard]`."""
    lines = []
    for name, _, variables in themes:
        entries = []
        for var in shared:
            if is_text_style(var) or (color := opaque_rgb(variables[var])) is None:
                continue
            rich = RichColor.from_rgb(color.r, color.g, color.b)
            row = [color.hex6, rich.downgrade(ColorSystem.EIGHT_BIT).number, rich.downgrade(ColorSystem.STANDARD).number]
            entries.append(f"      {json.dumps(var)}: {json.dumps(row)}")
        lines.append(f"    {json.dumps(name)}: {{\n" + ",\n".join(entries) + "\n    }")
    header = f'  "textual_version": {json.dumps(TEXTUAL_VERSION)},\n  "rich_version": {json.dumps(RICH_VERSION)},\n'
    DOWNGRADE.parent.mkdir(parents=True, exist_ok=True)
    DOWNGRADE.write_text("{\n" + header + '  "themes": {\n' + ",\n".join(lines) + "\n  }\n}\n")


def main() -> None:
    themes = [(name, theme, theme.to_color_system().generate()) for name, theme in BUILTIN_THEMES.items()]
    # Only the variables every theme defines become fields; that leaves out Textual's `surface-actrive` typo and a
    # few that only some themes set.
    shared = [name for name in themes[0][2] if all(name in variables for _, _, variables in themes)]
    if sys.argv[1:] == ["--downgrade"]:
        write_downgrade(themes, shared)
        return
    if sys.argv[1:]:
        fail(f"unknown arguments {sys.argv[1:]}; the only option is --downgrade")

    out = [
        f"// @generated by {SCRIPT} from textual=={TEXTUAL_VERSION}. Do not edit; regenerate with",
        f"// `uv run --frozen python {SCRIPT} > crates/lazy-github-tui/src/theme/palette.rs`.",
        "",
        "//! Textual's computed CSS variables for each built-in theme, in `BUILTIN_THEMES` order.",
        "",
        "use ratatui::style::Modifier;",
        "",
        "use super::color::{rgb, rgba, AnsiColor, TextStyle, ThemeColor, ThemeColor::*};",
        "",
        "/// One theme's colors and text styles: every variable Textual's `ColorSystem.generate` gives all built-in",
        "/// themes, named after the CSS variable (`$primary-lighten-3` is `primary_lighten_3`).",
        "#[derive(Debug, PartialEq)]",
        "pub struct Palette {",
    ]
    for name in shared:
        kind = "TextStyle" if is_text_style(name) else "ThemeColor"
        out.append(f"    /// `${name}`.")
        out.append(f"    pub {field(name)}: {kind},")
    out += [
        "}",
        "",
        "/// A built-in theme.",
        "pub(super) struct ThemeDef {",
        "    pub(super) name: &'static str,",
        "    pub(super) dark: bool,",
        "    pub(super) ansi: bool,",
        "    pub(super) palette: Palette,",
        "}",
        "",
        f"pub(super) static BUILTIN: [ThemeDef; {len(themes)}] = [",
    ]
    composites = {}
    for name, theme, variables in themes:
        out.append("    ThemeDef {")
        out.append(f"        name: {json.dumps(name)},")
        out.append(f"        dark: {str(theme.dark).lower()},")
        out.append(f"        ansi: {str(theme.ansi).lower()},")
        out.append("        palette: Palette {")
        for var in shared:
            out.append(f"            {field(var)}: {rust_value(var, variables[var])},")
        out.append("        },")
        out.append("    },")
        composites[name] = [
            {"fg": fg, "bg": bg, "result": composite_result(variables, fg, bg)} for fg, bg in COMPOSITE_PAIRS
        ]
    out.append("];")
    print("\n".join(out))

    COMPOSITES.parent.mkdir(parents=True, exist_ok=True)
    fixture = {"textual_version": TEXTUAL_VERSION, "themes": composites}
    COMPOSITES.write_text(json.dumps(fixture, indent=2) + "\n")


if __name__ == "__main__":
    main()
