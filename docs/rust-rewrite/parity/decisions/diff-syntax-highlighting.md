# Diff hunk panes are colored by syntect's grammars

- Decision: The diff viewer's hunk panes tokenize code with syntect's bundled grammars rather than Pygments' lexers
  with the monokai style, so some tokens get different colors than in Python. A file whose language has no grammar is
  drawn plain.
- Decided by: Chris, 2026-10-26
- Link: [#237 (comment)](https://github.com/gizmo385/gh-lazy/issues/237#issuecomment-6430749557)
- Rows: `diff-viewer.hunk-pane`

The same choice as [markdown-code-highlighting](./markdown-code-highlighting.md) for fenced code: Pygments has no Rust
equivalent, and the parity harness compares text, not colors. The row backgrounds (removed, added and current lines,
#236) don't depend on the tokenizer and match Python.

The grammars are bat's (`two-face`, PLAN §4), a superset of syntect's that adds TOML, Dockerfile and others, and
the markdown fences share them. The colors are those of the Pygments monokai style the Python app is locked to
(`Comment` and `Generic.Subheading` `#959077`, `Operator`, `Keyword.Namespace` and `Name.Tag` `#ff4689`), rather
than older Pygments' `#75715e` and `#f92672`.
