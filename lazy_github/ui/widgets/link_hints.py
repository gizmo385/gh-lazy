"""Hint-mode (Vimium-style) link follower for the main screen.

Press the bound key, see one-or-two-letter labels overlaid on every visible
link, type the label to activate. Works for both `[@click=...]` action markup
in Labels/Static widgets and standard Markdown links.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import TYPE_CHECKING

from textual import events
from textual.app import ComposeResult
from textual.content import Content
from textual.geometry import Offset
from textual.screen import ModalScreen
from textual.widget import Widget
from textual.widgets import Static

if TYPE_CHECKING:
    from lazy_github.ui.screens.primary import LazyGithubMainScreen

HINT_ALPHABET = "asdfghjklqwertyuiopzxcvbnm"

_MARKDOWN_LINK_ACTION_RE = re.compile(r"^link\((['\"])(?P<href>.*)\1\)$")
_OPEN_GH_LINK_ACTION_RE = re.compile(r"^screen\.open_gh_link\((['\"])(?P<href>.*)\1\)$")


@dataclass
class _LinkHit:
    """A clickable thing we found on screen, with the absolute position to anchor a hint to."""

    x: int
    y: int
    label_text: str
    action: str | None = None
    href: str | None = None


def _hint_codes(count: int, alphabet: str = HINT_ALPHABET) -> list[str]:
    """Generate `count` short, distinct hint codes (a, b, ..., aa, ab, ...)."""
    if count <= 0:
        return []
    if count <= len(alphabet):
        return list(alphabet[:count])
    # Two-char codes; should be enough for any realistic page.
    codes = list(alphabet)
    for first in alphabet:
        for second in alphabet:
            codes.append(first + second)
            if len(codes) >= count:
                return codes[:count]
    return codes[:count]


def _content_from_widget(widget: Widget) -> Content | None:
    """Pull the Content that a widget renders, if any."""
    visual = getattr(widget, "visual", None)
    if isinstance(visual, Content):
        return visual
    renderable = getattr(widget, "renderable", None)
    if isinstance(renderable, Content):
        return renderable
    internal = getattr(widget, "_content", None)
    if isinstance(internal, Content):
        return internal
    return None


def _extract_link_text(plain: str, span_start: int, span_end: int) -> str:
    text = plain[span_start:span_end].strip()
    return text or plain[max(0, span_start - 10) : span_end + 10].strip()


def _action_from_span_style(style: object) -> str | None:
    """Pull the click action string out of a span's style.

    Handcrafted markup stores it as a literal `@click=action` string;
    the Markdown widget stores it as a `Style` instance with `meta={"@click": action}`.
    """
    if isinstance(style, str):
        if style.startswith("@click="):
            return style[len("@click=") :]
        return None
    meta = getattr(style, "meta", None)
    if isinstance(meta, dict):
        action = meta.get("@click")
        if isinstance(action, str):
            return action
    return None


def _classify_action(action: str) -> tuple[str | None, str | None]:
    """Map a click action string to (action_to_run, href_to_open)."""
    if md := _MARKDOWN_LINK_ACTION_RE.match(action):
        return None, md.group("href")
    if gh := _OPEN_GH_LINK_ACTION_RE.match(action):
        return None, gh.group("href")
    return action, None


def collect_link_hits(screen_widget: Widget) -> list[_LinkHit]:
    """Walk the widget tree under `screen_widget` and return one hit per clickable link."""
    hits: list[_LinkHit] = []
    seen_positions: set[tuple[int, int]] = set()
    for node in screen_widget.walk_children(with_self=False):
        if not isinstance(node, Widget):
            continue
        if not node.display or not node.visible:
            continue
        region = node.region
        if not region or region.width <= 0 or region.height <= 0:
            continue

        content = _content_from_widget(node)
        if content is None or not content.spans:
            continue

        # Wrap to the widget's content width so span offsets become per-line.
        content_region = node.content_region
        wrap_width = content_region.width if content_region.width > 0 else region.width
        if wrap_width <= 0:
            continue

        try:
            wrapped_lines = content.wrap(wrap_width)
        except Exception:
            continue

        origin_x = content_region.x if content_region.width > 0 else region.x
        origin_y = content_region.y if content_region.height > 0 else region.y
        max_y = region.y + region.height

        for line_index, line in enumerate(wrapped_lines):
            line_y = origin_y + line_index
            if line_y >= max_y:
                break
            for span in line.spans:
                action_str = _action_from_span_style(span.style)
                if action_str is None:
                    continue
                action, href = _classify_action(action_str)
                prefix = line.plain[: span.start]
                col = Content(prefix).cell_length
                x = origin_x + col
                if x >= region.x + region.width:
                    continue
                key = (x, line_y)
                if key in seen_positions:
                    continue
                seen_positions.add(key)
                hits.append(
                    _LinkHit(
                        x=x,
                        y=line_y,
                        label_text=_extract_link_text(line.plain, span.start, span.end),
                        action=action,
                        href=href,
                    )
                )
    return hits


class LinkHintScreen(ModalScreen[_LinkHit | None]):
    """Transparent overlay that shows hint markers and captures the next keystrokes."""

    DEFAULT_CSS = """
    LinkHintScreen {
        background: transparent;
        layers: hints;
    }

    LinkHintScreen .link-hint {
        layer: hints;
        background: $warning;
        color: black;
        text-style: bold;
        height: 1;
        padding: 0;
        margin: 0;
        border: none;
    }
    """

    def __init__(self, hits: list[_LinkHit]) -> None:
        super().__init__()
        codes = _hint_codes(len(hits))
        # Pad all codes to a uniform width so single-letter hints don't get
        # stretched by layout and so two-letter hints aren't truncated.
        self._code_width = max((len(c) for c in codes), default=1)
        self._by_code: dict[str, _LinkHit] = {code: hit for code, hit in zip(codes, hits)}
        self._buffer = ""

    def compose(self) -> ComposeResult:
        for code, hit in self._by_code.items():
            hint = Static(code.ljust(self._code_width), classes="link-hint")
            hint.styles.width = self._code_width
            hint.absolute_offset = Offset(hit.x, hit.y)
            yield hint

    def on_key(self, event: events.Key) -> None:
        if event.key == "escape":
            event.stop()
            self.dismiss(None)
            return

        char = event.character
        if not char or char not in HINT_ALPHABET:
            return

        event.stop()
        self._buffer += char

        # Exact match → fire.
        if hit := self._by_code.get(self._buffer):
            self.dismiss(hit)
            return

        # No prefix matches anything → bail.
        if not any(code.startswith(self._buffer) for code in self._by_code):
            self.dismiss(None)


async def follow_link(screen: "LazyGithubMainScreen", *, external: bool = False) -> None:
    """Entry point: collect links on the main screen, show hints, dispatch the pick.

    With `external=True`, picked URLs open in the system browser instead of
    being routed through the in-app resolver. Non-URL action hits still run
    in-app since "open externally" has no meaning for them.
    """
    hits = collect_link_hits(screen)
    if not hits:
        screen.notify("No links visible", severity="information")
        return

    pick = await screen.app.push_screen_wait(LinkHintScreen(hits))
    if pick is None:
        return
    if pick.href is not None:
        if external:
            screen.app.open_url(pick.href)
            screen.notify(f"Opening {pick.href} externally")
        else:
            screen.action_open_gh_link(pick.href)
    elif pick.action is not None:
        await screen.run_action(pick.action)  # pragma: no cover - no current callers
