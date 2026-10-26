# Fenced code is colored by syntect's grammars, and unlabeled fences stay plain

- Decision: Fenced code is tokenized with syntect's bundled grammars rather than Pygments' lexers, so some tokens
  get different colors than in Python. A fence without a language is drawn plain `$text`, where Python guesses its
  language.
- Decided by: Chris, 2026-10-26
- Link: [#217 (comment)](https://github.com/gizmo385/gh-lazy/issues/217#issuecomment-6410261584)
- Rows: `markdown-links.code-highlighting`

Textual 8.2.5's `MarkdownFence.highlight` lexes a fence with the Pygments lexer named by its info string and styles
each token type through `HighlightTheme.STYLES` (`ANSIDarkHighlightTheme` / `ANSILightHighlightTheme` on ANSI
themes), a table from Pygments token types to theme variables. Pygments has no Rust equivalent, so the Rust renderer
(`crates/lazy-github-tui/src/components/markdown/highlight.rs`) parses with syntect, maps each scope to the Pygments
token type it's nearest to (`keyword` to `Keyword`, `string.quoted.double` to `Literal.String.Double`,
`support.function` to `Name.Builtin`, ...), and styles that through the same three tables, resolved against the
current theme. The colors therefore come from the same theme variables, but the grammars split and name code
differently. For example, syntect scopes `let` and `u8` in Rust alike (`storage.type`), where Pygments has `Keyword`
and `Keyword.Type`; Python identifiers outside a definition get no scope, where Pygments' `Name` colors them
`$text-primary`; Rust attributes are `variable.annotation`, where Pygments has `Comment.Preproc`. The tokens the
renderer's tests check against Textual (`echo hello` in `sh`, and a Python sample of a comment, keywords, strings,
`and`, `None` and names, on `textual-dark` and both ANSI themes) come out in Python's colors.

Python guesses the language of a fence without one with Pygments' `guess_lexer`, which scores every lexer against the
text. syntect has nothing like it, and a guess is often wrong for short snippets, so Rust draws those fences plain,
as it does indented code and fences in a language with no grammar.

The parity harness compares text, not colors, so the `repo-overview` scenario's README (an `sh` fence) reads the same
in both apps.
