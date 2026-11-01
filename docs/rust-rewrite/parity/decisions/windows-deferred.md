# Windows isn't supported in the first Rust release

- Decision: No Windows CI job, no `windows-amd64` release asset, and no support claim for 1.0. The code stays
  cross-platform where that's cheap (no Unix-only shortcuts without a `cfg`), and Windows can come back as its own
  issue later. Windows bugs reported before then are welcome, but they don't block a release.
- Decided by: Chris, 2026-10-31 ("We can punt on Windows support")
- Link: #202
- Rows: `infrastructure.windows-ci`

The Python app ran wherever Textual did, Windows included, so this is a difference from it. The release workflow
(#203) already leaves Windows out until #202 decides otherwise, and the cutover (#207) no longer waits on it.
