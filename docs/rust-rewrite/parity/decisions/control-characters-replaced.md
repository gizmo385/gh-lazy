# Control characters in GitHub text are drawn as U+FFFD

- Decision: Every control character (C0, DEL and C1) in text that comes from GitHub, `gh` or `git` is drawn as
  U+FFFD, everywhere the app shows such text, as the command log already does. Raw-text bodies from `gh` (diffs, job
  logs) are decoded lossily: invalid UTF-8 becomes U+FFFD instead of failing the request. ANSI escape sequences in job
  logs aren't interpreted: the ESC shows as U+FFFD and the rest of the sequence as text.
- Decided by: Chris, in the #327 spec (from #201's hardening rule) and the B34 answer (2026-10-30)
- Link: Py bug [B34](../py-bugs/B34.md), answered "follow the recommendation"; #327
- Rows: `main-layout.text-from-github`

Python leaves this to Textual and Rich. Rich strips BEL, BS, VT, FF and CR from a `Text`, Textual's `Log` replaces
U+0000–U+0014, and everything else reaches the terminal raw: an ANSI sequence in a PR title, a comment or a job log
recolors the rest of the row or moves the cursor, and the C1 CSI (U+009B) does the same in terminals that honour 8-bit
controls. Python also decodes `gh`'s output with a strict `.decode()`, so a diff of a binary file or a log with a stray
byte raises `UnicodeDecodeError`.

ratatui doesn't draw control characters at all, but measures them as one cell each, so before this decision a title
holding an ESC pushed every later column of its table row one cell left. Drawing U+FFFD keeps the row aligned and
shows that something was there, matching the command log (B34).

Python doesn't show a job log's ANSI colors either: it puts the log in a markdown code fence, which doesn't parse
escape sequences. So Rust doesn't color them; it only stops them from reaching the terminal.

Text without control characters or invalid UTF-8 looks the same in both apps, which the `unicode` scenario checks for
wide, combining and right-to-left text. Control characters are kept out of that scenario, since Python's output for
them depends on the terminal; unit tests and snapshots pin the Rust side.
