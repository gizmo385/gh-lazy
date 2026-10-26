# Markdown and links

Part of the [parity matrix](../README.md), which defines the columns, the status values and how to edit
this file.

| Id | Item | Python reference | Milestone | Status | Issue | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| `markdown-links.rendering` | Markdown rendering of bodies, comments, README, debug info | Textual `Markdown` | M2 | not started | #174 | PLAN §4: own pulldown-cmark renderer. |
| `markdown-links.reference-expansion` | Reference expansion: `#N`, `owner/repo#N`, `@user` become links, not inside code fences or inline code | `lib/github/references.py` `expand_github_references` | M2 | not started | | Port the regexes' lookbehind rules. |
| `markdown-links.link-click` | Clicking a markdown link or `@click` link: PR/issue URLs open in-app (repo switches), user URLs open the profile modal, everything else opens the browser | `ui/screens/primary.py` `action_open_gh_link`, `handle_markdown_link_clicked`, `_open_link_subject`; `ui/screens/link_paste.py` `resolve_github_url` (`accept_repo=False`) | M2 | not started | | Bare repo URLs go to the browser here, unlike the paste modal. |
| `markdown-links.link-hints` | Link hints overlay | `ui/widgets/link_hints.py` | M4 | not started | | |
| `markdown-links.hint-labels` | `f` shows hint labels (from `asdfghjklqwertyuiopzxcvbnm`, two letters when more than 26) on every visible link; typing a label follows it in-app; `F` opens it in the browser with toast "Opening <url> externally"; `escape` or a non-matching prefix cancels; toast "No links visible" | `ui/widgets/link_hints.py` `follow_link`, `LinkHintScreen`, `collect_link_hits` | M4 | not started | | |
| `markdown-links.paste-link-result` | Paste link modal (see [Modals](./modals.md)) result: PR → select repo and PR, toast "Opening pull request for link"; issue → "Opening issue for link"; repo → "Opening repository for link"; user → profile modal | `ui/screens/primary.py` `action_open_link_paste_modal`, `_open_link_subject` | M4 | not started | | TERM: `ctrl+v`. |
