# Parity fixtures and fake-gh

Both LazyGithub implementations reach GitHub only through the `gh` CLI. For parity runs and tests, `crates/fake-gh` stands
in for `gh` and answers from the fixture files in `parity/fixtures/`, so the Python and Rust apps see byte-identical
data with no network. See `docs/rust-rewrite/PLAN.md` §6.2.

- [fake-gh](#fake-gh): installing it, environment variables, supported commands, output, the request log.
- [Fixture format (v1)](#fixture-format-v1): layout, file schema, key normalization, matching.
- [`fake-gh lint`](#fake-gh-lint).
- [Recording fixtures](#recording-fixtures): `cargo xtask record-fixtures`, the scope rules, the scrub rules and
  `cargo xtask check-fixtures`.
- [The parity harness](#the-parity-harness-cargo-xtask-parity): running scenarios, flags, the isolated environment,
  the scenario format, keeping the suite fast, screen dumps, structure extraction, the checks and results.

## fake-gh

`fake-gh` is a drop-in `gh` binary. Build it with `cargo build -p fake-gh`, install `target/debug/fake-gh` as
`<tmp>/bin/gh` (`gh.exe` on Windows), and put that directory first on `PATH`. From Rust, `fake_gh::install(binary,
dir)` does the install: it hard-links when it can, and otherwise copies and waits out the `ETXTBSY` race that a
freshly copied executable has in a multi-threaded process. The binary is deliberately not named
`gh`, so `target/debug` on `PATH` never shadows the real gh.

The crate builds a second binary, `fake-ssh`, that `fake_gh::install` puts next to `gh` as `<dir>/bin/fake-ssh`
whenever it's in the same build dir as fake-gh (`cargo build -p fake-gh` builds both). See [fake-ssh](#fake-ssh).

### Environment variables

fake-gh reads its configuration from the environment only.

| Env var | Meaning | Default |
| --- | --- | --- |
| `FAKE_GH_FIXTURES` | Fixture set directories, joined with the platform path separator (`:` on Unix, `;` on Windows). An earlier directory overrides a later one. | Required by `gh api` and `gh pr diff`, which exit 3 without it. `auth`, `version` and `fake-gh lint` don't need it. |
| `FAKE_GH_LOG` | Path of the JSONL request log, appended to on every invocation. | Unset: no log, and `when` conditions are always false. |
| `FAKE_GH_AUTH` | `logged-in` or `logged-out`. Anything else exits 3. | `logged-in` |
| `FAKE_GH_USER` | Login shown by `gh auth status`. | `parity-bot` |

### Supported commands

Flags may come before or after the subcommand, as with gh's cobra parser: `gh --method PATCH api /x` works. Long flags
accept `--flag value` and `--flag=value`; short flags accept `-X GET`, `-XGET` and combined booleans (`-iX GET`).

1. **`gh api [flags] <endpoint>`**
   - `-X/--method <M>`: defaults to `GET`. Without it, the method is `POST` when any `-f`/`-F` field or `--input` is
     given, or when the endpoint is `graphql` (gh's rule).
   - `-H/--header "Name: value"`: repeatable. Only `Accept` affects matching (the last one wins). `Cache-Control` is
     logged.
   - `-F/--field k=v` and `-f/--raw-field k=v`: for `GET` (and whenever `--input` supplies the body, as in gh) they
     become query params. Otherwise they build a JSON object body. `-F` converts `true`/`false`/`null`/integers and reads
     `@file` (`@-` is stdin); `-f` keeps strings. `k[]=v` appends to an array. On the query, typed values are formatted
     back the way gh sends them: `-F all=false` gives `"false"`, `-F c=null` gives `""`, `-F page=007` gives `"7"`. For
     `graphql`, fields other than `query` and `operationName` go under `variables`.
   - `--input <file>|-`: the request body, read from the file or stdin. It's parsed as JSON for matching when it
     parses, and kept as a string otherwise.
   - `-i/--include`: print the status line and headers before the body.
   - `--cache <dur>`: accepted and logged as `cache`. Nothing is cached.
   - `--paginate` and `--slurp`: follow `Link: <url>; rel="next"` from fixture to fixture. A missing next page is a
     miss. The pages are printed as gh 2.89 prints them:
     - Without `-i` or `--slurp`, when every page is a JSON array, the arrays are merged into one (`[a,b]` and `[c]`
       give `[a,b,c]`, as gh's `paginatedArrayReader` does). Other pages (objects, text) are printed back to back.
     - With `-i`, page bodies are printed back to back, each with its own status line and headers.
     - With `--slurp`, the pages are wrapped in one outer JSON array.
   - `--hostname github.com`: accepted. Any other host exits 2.
   - Any other flag (`--jq`, `--template`, `--verbose`, `-p`, ...) exits 2 with `fake-gh: unsupported flag --jq` on
     stderr, and is logged with `"error"`.
   - `graphql` as the endpoint means `POST /graphql`, matched on the body like any POST.
   - gh's `{owner}`/`{repo}`/`{branch}` placeholders exit 2.
2. **`gh auth status`**, with `--hostname`, `--active` and `-t` accepted.
   - Logged in: exit 0 and, on stdout:
     ```
     github.com
       ✓ Logged in to github.com account <FAKE_GH_USER> (keyring)
       - Active account: true
       - Git operations protocol: https
       - Token: gho_************************************
     ```
   - Logged out: exit 1, stderr `You are not logged into any GitHub hosts. To log in, run: gh auth login`.
3. **`gh auth token`**: prints `fake-gh-token`, or exits 1 when logged out. The value deliberately doesn't look like a
   real token, so secret scanners stay quiet.
4. **`gh --version` / `gh version`**: prints `gh version 2.89.0 (fake-gh)` and
   `https://github.com/cli/cli/releases/tag/v2.89.0`. Tools can look for `fake-gh` in this output to detect it.
5. **`gh pr diff [-R|--repo] <owner>/<repo> <n> [--color never|auto|always] [--patch]`**: looked up as
   `GET /repos/<owner>/<repo>/pulls/<n>` with accept `diff` (`patch` with `--patch`). Prints only the body, exit 0. On
   a miss or status ≥ 400, prints `could not find pull request diff: HTTP <status>` on stderr and exits 1. `-R` is
   required, since fake-gh doesn't read git remotes.
6. Anything else exits 2 with `fake-gh: unsupported command: gh <argv>` on stderr, and is logged as an error.

**Known differences from real gh.** Neither app sends these shapes, so they're left as is:

- A bare `gh api graphql` (no fields, no `--input`) is a POST here; real gh sends a GET. #67 chose POST, and real
  GraphQL calls always carry `-f query=`.
- `--input` files and `-F k=@file` values must be UTF-8 text. A non-UTF-8 file, or a missing one, exits 2 (real gh
  sends the bytes, and exits 1 on a missing file).
- Boolean flags don't take a value: `--paginate=true` or `--include=false` exits 2, where real gh's pflag accepts them.

Exit codes: 0 success; 1 HTTP status ≥ 400, a miss, or logged out; 2 unsupported argv; 3 fake-gh configuration problems
(no `FAKE_GH_FIXTURES`, a fixture that doesn't load, duplicate matchers, an ambiguous match, log I/O errors).

### Output of `gh api`

- With `-i`: `HTTP/2.0 <status> <reason>\n` (LF only, as real gh prints it), using the canonical reason phrase from the
  `http` crate. Then each response header, sorted by name, with Go-canonical names (`Link`, `Etag`,
  `X-Ratelimit-Limit`), as `Name: value\r\n`. Then `\r\n`, then the body.
- fake-gh always prints `HTTP/2.0`: the Python app only recognizes a status line starting with `http/2.0`.
- The body is printed as-is with no trailing newline added. A JSON `body` is serialized compactly on one line with key
  order preserved. `body_text` and `body_file` are printed byte for byte, so `body_file` can be binary (for example a
  logs zip). Redirects are not modeled: the fixture holds the final response.
- `Content-Type` is added when the fixture omits it: `application/json; charset=utf-8` for `body`,
  `text/plain; charset=utf-8` otherwise.
- Status < 400: exit 0. Status ≥ 400: the body still goes to stdout, then stderr gets `gh: <body.message> (HTTP <status>)`
  or `gh: HTTP <status>` when the body has no `message`, and the exit code is 1. This matches real gh.
- **Miss** (no fixture): status 404 with body
  `{"message":"Not Found","documentation_url":"https://docs.github.com/rest","status":"404"}` and exit 1, which is what
  real GitHub returns for an unknown path. An absolute URL on a host other than `api.github.com` is a miss too. The log
  line gets `"miss": true`. Failing a run that has misses is the harness's job; fake-gh itself never fails the app in a
  way real GitHub wouldn't.
- `delay_ms` in a fixture sleeps before printing (after the log line is written), so scenarios can capture loading
  states.

### Request log (`FAKE_GH_LOG`)

One JSON object per invocation, one per line, appended. The file is held under an exclusive lock
(`std::fs::File::lock`) from the read that `when` conditions need until the append, because the apps run several `gh`
processes at once.

```json
{"seq":3,"argv":["api","-i","-X","GET","-F","page=1","/user/repos"],"command":"api","method":"GET","path":"/user/repos","query":{"page":"1"},"accept":"json","cache":null,"cache_control":"max-age=86400","body":null,"fixture":"base/user/repos/GET.page-1.json","status":200,"miss":false,"error":null}
```

| Field | Meaning |
| --- | --- |
| `seq` | 1-based line number at the moment of writing. |
| `argv` | The arguments, without the program name. |
| `command` | `api`, `auth status`, `auth token`, `version` or `pr diff`. `null` when argv couldn't be parsed. |
| `method`, `path`, `query`, `accept` | The normalized request key (`api` and `pr diff` only). |
| `cache`, `cache_control` | The `--cache` value and the last `Cache-Control` header. |
| `body` | The request body: JSON when it parsed, a string otherwise. |
| `fixture` | The fixture served, relative to the parent of its set directory, with `/` separators. |
| `status` | The HTTP status answered (the last page's, with `--paginate`). |
| `miss` | No fixture matched. With `--paginate`, the key logged is the page that missed, so it can be recorded. |
| `error` | Why fake-gh refused the invocation (exit 2 or 3). |

[fake-ssh](#fake-ssh) appends its own lines to the same log, under the same lock, so a report shows the pushes next
to the API calls:

```json
{"seq":7,"argv":["git@github.com","git-receive-pack 'parity-bot/pr-details.git'"],"command":"ssh","service":"git-receive-pack","method":null,"path":"parity-bot/pr-details.git","query":null,"accept":null,"cache":null,"cache_control":null,"body":null,"fixture":null,"status":0,"miss":false,"error":null}
```

`command` is `ssh`, `service` the git service (`git-upload-pack` or `git-receive-pack`; `null` when the command
was refused before it was known), `path` the repo path under `FAKE_SSH_ROOT`, and `status` fake-ssh's exit code.
The other fields are `null` (`miss` is `false`); fake-gh's lines have no `service`. `ssh` lines are never served
requests: `when` conditions and `wait.log` ignore them, and so does the served-requests comparison of references.

### fake-ssh

`fake-ssh` stands in for `ssh` when git reaches `git@github.com:<owner>/<name>.git`, and serves the request from a
local bare repo instead. The harness runs the app with `GIT_SSH=<run>/bin/fake-ssh` and `GIT_SSH_VARIANT=simple`, so
git runs it as `fake-ssh [-p <port>] <host> <command>`, where `<command>` is one argument: the service, a space, and
the repo path in single quotes. fake-ssh:

- accepts `git-upload-pack` (fetch, `ls-remote`) and `git-receive-pack` (push). Anything else exits 128 with
  `fake-ssh: unsupported command: <command>` on stderr;
- strips the quotes and a leading `/` from the path and resolves it under `FAKE_SSH_ROOT`. A path that leaves the
  root (`..`, a symlink) or names no directory exits 128 with `ERROR: Repository not found.` on stderr, as GitHub
  answers;
- runs `git upload-pack <dir>` / `git receive-pack <dir>` (no shell) with stdin, stdout and stderr passed through,
  and exits with its code. The host and port are ignored;
- when `FAKE_GH_LOG` is set, appends one `ssh` line to the request log (above).

## Fixture format (v1)

This is the contract between fake-gh, the recorder (`cargo xtask record-fixtures`, #68) and the harness (#69). The
serde types live in `fake_gh::fixture`.

### Layout

`parity/fixtures/<set>/` is one fixture set: `base`, or a scenario overlay like `pr-merge`. Inside a set, each fixture is
one JSON file at a path that mirrors the URL path:

```
parity/fixtures/base/user/GET.json
parity/fixtures/base/user/repos/GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json
parity/fixtures/base/repos/gizmo385/gh-lazy/readme/GET.raw.json
parity/fixtures/base/repos/gizmo385/gh-lazy/pulls/12/GET.diff.json
parity/fixtures/base/repos/gizmo385/gh-lazy/pulls/12/GET.diff.body.diff     (body_file sibling)
parity/fixtures/pr-merge/repos/gizmo385/gh-lazy/pulls/12/merge/PUT.json
```

- File name: `<METHOD>[.<accept>][.<query-slug>][.<n>].json` (`fake_gh::fixture::file_name_for`).
  - `<accept>` appears only when it isn't `json`.
  - `<query-slug>` is the canonical query as `k-v` pairs in key order, joined by `_`. In each key and value, any
    character outside `[A-Za-z0-9._-]` is replaced by `-`. A slug longer than 80 characters is cut to 80 and gets
    `-<hash>` appended, where `<hash>` is 4 hex digits of FNV-1a over the full `k=v&...` query.
  - `.<n>` (2, 3, ...) tells apart files that would otherwise get the same name, e.g. fixtures that differ only in
    `body` or `when`.
  - Directory segments containing `<>:"\|?*` (or `%`) are percent-encoded so the tree works on Windows.
- **Names are for humans only.** The loader reads every `*.json` under each set (except `*.body.*` sibling files) and
  takes the key from the `request` object inside. Hand-written fixtures can be named freely, but the recorder and
  `fake-gh lint` hold names to the convention.

### File schema

```jsonc
{
  "version": 1,
  "request": {
    "method": "GET",                      // uppercase
    "path": "/user/repos",                // leading slash, no host, no query, no trailing slash, percent-decoded
    "query": {"direction": "asc", "page": "1", "per_page": "50", "sort": "full_name", "type": "all"},
                                          // string -> string | [string]; omitted = {}
    "accept": "json",                     // normalized, see below; omitted = "json"
    "body": {"merge_method": "squash"},   // optional matcher: the request body must equal this JSON
    "when": {"after": [{"method": "PUT", "path": "/repos/o/r/pulls/12/merge"}]}
                                          // optional: only matches once the log holds a served request matching each entry
  },
  "response": {
    "status": 200,
    "headers": {"Link": "<https://api.github.com/user/repos?page=2&per_page=50>; rel=\"next\""},
    "body": [ /* JSON value */ ],         // at most one of body | body_text | body_file; none = empty body
    "body_text": "raw text",
    "body_file": "GET.diff.body.diff",    // a sibling file (relative to this file's directory), for big diffs and logs
    "delay_ms": 0
  },
  "note": "free text, ignored"
}
```

- Unknown fields are rejected, so a typo like `"qurey"` is a load error rather than a silently broader matcher.
- Query values are strings (or arrays of strings), as they are on the wire: write `"page": "1"`, not `"page": 1`.
- `body_file` is a bare file name in the fixture's own directory; `lint` rejects paths like `../x` or `/abs`.
- The loader doesn't follow symlinked directories (so a loop can't hang it); a symlink to a file is read.
- `request.body: null` is the same as no `body` matcher.
- A `when.after` entry has `method` and `path`, and optionally `query`, `accept` and `body`, which are compared only when
  given. A "served" request is a log line with a `fixture`, no `miss` and no `error`.
- Header names can be written in any case; fake-gh prints them Go-canonicalized.

### Key normalization

fake-gh applies this to incoming requests, and the recorder applies it to recorded ones (`RequestKey::from_url`):

- Path: strip `https://api.github.com` (any other host is a miss), split off `?query` and merge it with the `-F`/`-f`
  fields (URL query first, then fields in order), percent-decode, then ensure a leading `/` and no trailing `/`.
- Query: keys are compared exactly. Values are the strings gh would send. A key repeated in the request becomes an array
  in request order. Map comparison ignores key order. In the URL's own query, `+` decodes to a space.
- Paging: a GET whose query has `per_page` but no `page` is treated as `page=1` (gh `--paginate` sends the first page
  without `page`; Python sends `page=1`).
- Accept (`Accept::normalize`), the last `Accept` header winning, compared case-insensitively:
  - missing, `application/vnd.github+json`, `application/vnd.github.v3+json` or `application/json` → `json`
  - `application/vnd.github.diff` or `application/vnd.github.v3.diff` → `diff`
  - `application/vnd.github.raw`, `.v3.raw`, `.raw+json` or `.v3.raw+json` → `raw`
  - `application/vnd.github.patch` or `.v3.patch` → `patch`
  - anything else → the literal string
- Other headers never affect matching.

### Matching

A request is a candidate for every fixture with equal `method`, `path`, `query` and `accept`, where the fixture's `body`
(if present) equals the request body after JSON parsing and the fixture's `when` (if present) holds.

- The winner is the candidate with the highest score: +2 for `when`, +1 for `body`.
- A tie between sets goes to the earliest set in `FAKE_GH_FIXTURES`. A tie inside one set is an error.
- Two fixtures in one set with identical matchers (key, `body` and `when`) are a load error: exit 3, naming both files
  on stderr. An ambiguous tie found at request time (for example two different `when`s that both hold) also exits 3.

## `fake-gh lint`

```
fake-gh lint parity/fixtures/*
```

Available when the binary is invoked as `fake-gh` (not as `gh`). It loads each set, then reports load errors,
duplicate matchers, methods that aren't uppercase, paths that aren't normalized, `body_file`s outside the fixture's
directory, and files whose directory or name isn't the canonical one for their key. It exits 1 when there are problems. `cargo test` runs the same checks on every
committed set (`crates/fake-gh/tests/lint_fixtures.rs`).

## The `base` set

`parity/fixtures/base/` is hand-written for the fictional user `parity-bot` (id `1000001`):

- `GET /user`
- `GET /user/repos` with the Python app's page-1 query (`type=all`, `direction=asc`, `sort=full_name`, `page=1`,
  `per_page=50`): three public repos.
- `GET /notifications` with `all=false`, `page=1`, `per_page=50`: no notifications.

User-scoped endpoints stay hand-written so recordings never leak a real account's private data.

Repo-scoped fixtures in `base` are recorded from the public `gizmo385/gh-lazy` repo by `cargo xtask record-fixtures
--set base`, from `parity/fixtures/base/recipe.toml`. See
[Recording fixtures](#recording-fixtures).

## Overlays

- `repos-two-pages/`: base's three repos split over two `GET /user/repos` pages (two, then one) linked by
  `Link: rel="next"`. Listed before `base`, it exercises pagination; `crates/fake-gh/tests/rust_client.rs` uses it.
- `repos-60/`: 60 public repos owned by `parity-bot` and `parity-org`, with names long enough that the repos table
  overflows its pane, over two `GET /user/repos` pages (50, then 10). Listed before `base`, for the
  `table-navigation` and `table-search` scenarios: on the first page, three `parity-bot` names contain `lazy` and two
  start with `gh`. Hand-written: only the fields the apps read are kept.
- `repos-error/`: base's `GET /user/repos` page 1 answered with a 500 (`{"message": "Server Error"}`) after 1.5 s,
  long enough to capture the loading overlay. Listed before `base`, for the `repos-fetch-error` and
  `repos-cache-error` scenarios. Hand-written.
- `repos-slow/`: base's `GET /user/repos` page 1 answered after 2 s (`delay_ms = 2000`), so what the apps show before
  the list arrives (a seeded `repos.json`, its warning) can be captured. Listed before `base`, for the `repos-cache`
  and `repos-cache-corrupt` scenarios. Hand-written.
- `notifications-0/`, `notifications-3/`, `notifications-50/`: `GET /notifications` (`all=false`, `page=1`,
  `per_page=50`) answered with 0, 3 or 50 unread notifications (50 is a full first page) for repos in `base`. Listed
  before `base`, for the `status-bar-unread-*` scenarios. Hand-written.
- `notifications-modal/`: for the Notifications modal scenarios (`notifications`, `notifications-mark-all`,
  `notifications-open`; #189, #190). `GET /notifications` `all=true` page 1 (`per_page=50`, as both tabs page): six
  notifications in `parity-bot/pr-details` and `parity-bot/dotfiles`, four unread (9100001 PR 5 `review_requested`,
  9100002 issue 7 `mention` with a padded title, 9100004 a `ci_activity` CheckSuite with a `null` subject URL, 9100006
  PR 4 `state_change`) and two read (9100003 a Release, 9100005 PR 2, which 500s in `pr-details`), 9100002 listed
  before 9100003 though it's older, so the table's sort shows; then an empty page 2. `all=false` page 1 (the four
  unread, also the status bar's count) and an empty page 2. `PATCH /notifications/threads/<id>` (205) for the unread
  four and `PUT /notifications` with `{"read": "true"}` (205). Once `PATCH .../9100001` was served (`when.after`),
  `all=false` page 1 drops it; once the `PUT` was, `all=false` page 1 is empty and `all=true` page 1 has all six read.
  The two `when` variants never both hold in one scenario. Listed before `pr-details`, `repo-selected-empty` and
  `base`. Hand-written.
- `notifications-slow/`: `notifications-modal`'s `all=true` page 1 answered after 1.5 s (`delay_ms = 1500`), for the
  `notifications` scenario's loading capture. Listed before `notifications-modal`. Hand-written.
- `repo-selected-empty/`: what selecting `parity-bot/dotfiles` loads (issues, workflows, runs, the README), all empty
  but the README. Listed before `base`, for `status-bar-current-repo`. Hand-written.
- `repo-overview/`: for the `repo-overview` scenario (#219). Base's `GET /user/repos` page with `parity-bot/lazy-demo`
  archived and without a description; READMEs for `parity-bot/hello-world` (most CommonMark and GFM blocks,
  two tables, served after 1.5 s so the loading state can be captured), `parity-bot/lazy-demo` (a 404) and
  `parity-bot/dotfiles` (blank lines and a header-shaped line, Py bug B47); empty issues, workflows and runs for
  hello-world and lazy-demo. Listed before `repo-selected-empty` (dotfiles' issues and workflows) and `base`.
  Hand-written.
- `pr-details/`: the repo `parity-bot/pr-details`, listed first on base's `GET /user/repos` page, for the
  `pr-details`, `pr-diff` and `pr-lookup` scenarios (#230). Its issues page 1 (`state=all`) has PRs 5 (open), 4
  (closed), 3 (merged) and 2 (draft), and page 2 is empty. The full PRs 5 (served after 1 s), 4, 3 and 9 (not on the
  issues page, for the lookup), a 500 for PR 2 and a 404 for PR 42; each PR's commit status and check runs
  (`page=1`, `per_page=100`), reactions, reviews and review comments, issue comments and comment reactions (also
  the 404s Python gets asking for review comments' reactions on the issue-comment endpoint, Py bug B54); and the
  diffs (`GET .../pulls/<n>` with accept `diff`, which `gh pr diff` reads too): PR 5's five files, PR 3's and PR
  9's small ones, and a 406 for PR 4. Plus its README and empty workflows and runs. Listed before `base`.
  Hand-written, generated once by a throwaway script; edit the JSON directly.
- `prs-70/`: the repo `parity-bot/lazy-fixture`, added to base's `GET /user/repos` page, with 70 pull requests over
  `GET /repos/parity-bot/lazy-fixture/issues` pages of 30 (`state=all`, as `list_issues` sends them: 30, 30, 10, then
  an empty page 4), plus its README and empty workflows and runs. Listed before `base`, for the `table-lazy-load`
  scenario. Hand-written.
- `repo-lists/`: the repo `parity-bot/browse-fixture`, listed first on base's `GET /user/repos` page, with numbers 100
  to 36 over `GET /repos/parity-bot/browse-fixture/issues` pages of 30 (`state=all`: 30, 30, 5, then an empty page 4).
  A number divisible by 3 is a pull request (it has `draft` and `pull_request`) and every other number an issue: 10
  PRs and 20 issues on pages 1 and 2, then PRs 39 and 36 and three issues on page 3. PR 96 is closed, PR 84 closed
  and merged, PR 93 a draft, issues 98 and 80 closed; PR 90's title is `[WIP] Browse fixture pull request 90`. Issue
  authors alternate `parity-bot` and `parity-friend`. Plus its README and empty workflows and runs. Listed before
  `base`, for the `repo-lists` scenario. Hand-written.
- `pr-filters/`: `parity-bot/browse-fixture`'s issues pages for `state=open&creator=parity-bot`: the 40 open items by
  `parity-bot` from `repo-lists` (15 PRs and 15 issues on page 1, PRs 48 to 36 and five issues on page 2, then an
  empty page 3). Listed before `repo-lists` and `base`, for the `pr-filters` scenario. Hand-written, generated once by
  a throwaway script; edit the JSON directly.
- `linkify-diff-error/`: `parity-bot/pr-details`' README replaced by one paragraph of bare URLs, `www.` and fuzzy
  hosts and emails (no blank lines or `Word:` lines, which Python drops: Py bug B47), and PR 3's diff by one whose
  `---` line isn't followed by `+++`, which neither app parses. Listed before `pr-details` and `base`, for the
  `linkify-diff-error` scenario. Hand-written.
- `lists-error/`: `parity-bot/dotfiles`' `issues` page 1 (`state=all`), `actions/workflows` page 1 and `actions/runs`
  page 1, each answered with a 500 (`{"message": "Server Error"}`) after 1.5 s. Listed before `repo-selected-empty`
  (the README) and `base`, for the `lists-fetch-error` scenario. Hand-written.
- `workflow-runs/`: the repo `parity-bot/ci-fixture`, listed first on base's `GET /user/repos` page, with 38 workflow
  runs, newest first, over `GET /repos/parity-bot/ci-fixture/actions/runs` pages of 30 (30, 8, then an empty page 3)
  and three workflows (`CI`, `Lint`, `Release`) on `actions/workflows` page 1. Run numbers are unique across the
  workflows (CI from 101, Lint from 201, Release from 301). The newest run, `Ship the workflow runs table`, is
  `in_progress` and the next `queued`, both with a null `conclusion`; the rest mix `success`, `failure`, `cancelled`
  and `skipped`. On page 1 the 5th and 6th runs share a minute, the earlier second listed first, which pins the sort to
  the `Time` string. Plus its README and an empty issues page 1. Listed before `base`, for the `workflow-runs`
  scenario. Hand-written.
- `repos-tracked/`: `GET /repos/<owner>/<name>` for tracked repos (`repositories.additional_repos_to_track`):
  `parity-org/tracked-tool`, `parity-bot/hello-world` (also in base's list, with another description),
  `ghost-org/orphan` (whose owner 404s in `user-profile`) and `parity-org/gone` (a 404). Listed before `base`, for the
  `repos-tracked` and `user-profile` scenarios. Hand-written.
- `repo-lookup/`: `GET /repos/parity-org/lookup-me` (found) and `GET /repos/parity-org/missing` (a 404), plus what
  selecting `lookup-me` loads (empty issues, workflows and runs, a short README). Listed before `base`, for the
  `repo-lookup` scenario. Hand-written.
- `user-profile/`: `GET /users/<login>` and `GET /users/<login>/repos` (`sort=updated`, `direction=desc`,
  `per_page=30`) for `parity-bot` (every profile field, four repos, one of them `parity-bot/profile-only`, which isn't
  in base's list), `parity-org` (an organization with no name, bio or repos) and `ghost-org` (both 404), plus what
  selecting `profile-only` loads. Listed before `repos-tracked` and `base`, for the `user-profile` scenario.
  Hand-written.
- `table-caches/`: base's `GET /user/repos` page with `parity-bot/cache-fixture` first; its issues page 1 (three PRs,
  three issues), runs page 1 (three runs), empty second pages, no workflows and a README. Once
  `parity-bot/dotfiles`' issues have been served (`when.after`), its issues and runs page 1 answer 500, so selecting it
  again leaves the rows restored from the cache. Listed before `repo-selected-empty` and `base`, for the
  `table-caches` scenario. Hand-written.
- `issue-details/`: the repo `parity-bot/issue-fixture`, listed first on base's `GET /user/repos` page, with its
  README, empty workflows and runs, and `GET /repos/parity-bot/issue-fixture/issues` page 1 (`state=all`): issues 12
  (open; a body with `#N`, `owner/repo#N` and `@user` references, inline code, a fence and a quote), 11 (closed, no
  body), 10, 9 and pull request 8, then an empty page 2. Each issue's `GET .../issues/<n>/comments` (as Python
  requests `comments_url`, no query): three comments on 12 (one with a `null` user), none on 11, one on 10 after
  2 s, a 500 on 9 (it crashes Python: Py bug B60). `GET .../issues/<n>` for 3 (not on the listed page), 8 (a pull
  request) and 12, and a 404 for 404. Listed before `base`, for the `issue-details` and `issue-lookup` scenarios.
  Hand-written.
- `run-details/`: the repo `parity-bot/run-fixture`, listed first on base's `GET /user/repos` page, with its README,
  empty issues, one workflow and three runs of it on `actions/runs` page 1 (then an empty page 2): 42 (completed,
  failure, attempt 2), 41 (in progress) and 40 (completed, success). `GET .../actions/runs/<id>/jobs` (no query):
  four jobs with steps on 42 (success, failure, a skipped job and a cancelled one), two on 41 (one in progress, one
  queued) and none on 40. `GET .../actions/jobs/<id>/logs` (plain text, every line timestamped, so Py bug B47 doesn't
  show): a 404 for 42's skipped job and 41's queued job. Listed before `base`, for the `run-details` scenario.
  Hand-written.
- `issue-write/`: write requests for `parity-bot/issue-fixture`, listed before `issue-details` and `base`, for the
  `issue-new`, `issue-edit` and `issue-comment` scenarios (M3, #181, #182). Each write matches only the body its
  scenario sends: `POST .../issues` creates issue 13 after 1 s (and `GET .../issues/13/comments` has none),
  `PATCH .../issues/12` renames and closes issue 12 after 1.5 s, and `POST .../issues/12/comments` answers a new
  comment for each of the two bodies `issue-comment` posts. Hand-written.
- `pr-write/`: write requests for `parity-bot/pr-details`, listed before `pr-details` and `base`, for the `pr-merge`,
  `pr-reactions`, `pr-comment` and `pr-review` scenarios (M3, #182, #183, #187). `POST .../pulls/5/reviews` for the
  review `pr-review` submits; `PUT .../pulls/5/merge` for the squash method and PR 5's
  head sha (merged), `GET .../pulls/5` merged once that was served (`when.after`, for Rust's refetch), `PUT
  .../pulls/4/merge` (405); `POST .../issues/5/reactions` answering 201 for five kinds; `POST
  .../pulls/5/comments/9001/replies` and `POST .../issues/5/comments` for the bodies `pr-comment` posts, with
  `GET .../issues/5/comments` and `GET .../pulls/5/reviews/501/comments` including the new comments once they were
  posted, and the new comments' reactions (the 404 Python gets on the issue-comment path for the review reply, B54).
  Hand-written.
- `pr-create/`: for `parity-bot/pr-details`, listed before `pr-details` and `base`, for the `pr-new` and
  `pr-new-cwd` scenarios and the `pr-new-push` scenario of #280 (M3, #184). The branches `main`, `new-feature` and
  `next-release` (`page=1`, `per_page=30`), the collaborators (Python asks for them when Create PR opens), and
  `POST .../pulls` for four bodies: head `missing-branch` (422), the draft from `new-feature` with a two-line body
  (201, PR 10, after 1 s), and the non-draft from `new-feature` with no body (422, then 201 once the 422 was served,
  `when.after`). PR 10 itself, its diff (empty), reviews, issue comments, reactions and the status and check runs of
  its head sha. Hand-written.
- `workflow-write/`: for `parity-bot/run-fixture`, listed before `run-details` and `base`, for the `workflow-trigger`
  and `workflow-rerun` scenarios (M3, #188). Two workflows on `actions/workflows` page 1 (CI 7000001, Release
  7000002), the branches `main`, `release` and `run-details` (`page=1`, `per_page=30`), `POST
  .../workflows/7000002/dispatches` for `release` (204) and `.../workflows/7000001/dispatches` for `main` (422),
  `POST .../runs/5000042/rerun` and `.../rerun-failed-jobs` (201), and `GET .../runs/5000042` (for Rust's refetch;
  Python never asks). Hand-written.

## Seeds

A seed is a directory `parity/seeds/<name>/` holding a `home/` tree, a `work/` tree or both, named by a scenario's
`[env] seed` (see [the isolated environment](#the-isolated-environment)). `home/` mirrors the test `HOME`, `work/` the
app's working directory. The root `.gitignore` ignores `.cache` everywhere, so `!/parity/seeds/**/.cache/` re-includes
the seeds' cache directories.

- `repos-cache/`: `home/.config/lazy-github/.cache/repos.json` in Python's compact format with two repos,
  `parity-bot/cached-only` (only in the cache) then `parity-bot/dotfiles`. For the `repos-cache` and
  `repos-cache-error` scenarios.
- `repos-cache-corrupt/`: a `repos.json` cut off inside its first entry, so it isn't valid JSON. For the
  `repos-cache-corrupt` scenario.
- `pr-draft/`: a saved draft for `parity-bot/pr-details`
  (`home/.config/lazy-github/.cache/pr_drafts/parity-bot_pr-details.json`: title, body, base and head, draft, one
  reviewer) and a `work/pr_template.md`. For #186's draft-restore scenario.
- `pr-draft-corrupt/`: the same paths with a draft that has only `repo_full_name` and `title`, so it fails the
  schema. For #186's corrupt-draft scenario.

## Recording fixtures

```
cargo xtask record-fixtures --set <name> [--recipe <path>] [--from-log <fake-gh.log>]... [--gh <path>] [--check] [--dry-run]
cargo xtask check-fixtures [<fixtures-dir>]
```

`record-fixtures` runs a set's recipe against the real, authenticated `gh` and writes each response, scrubbed, as a
fixture. It's a developer tool, run by hand or by an agent. It never runs in CI, and it never sends anything but `GET`.

### Workflow

1. Run the parity scenarios against the Python app only (`cargo xtask parity --python-only`, #69). Every request
   fake-gh couldn't answer is a `"miss": true` line in
   `parity/reports/latest/<scenario>/<size>/<app>/fake-gh.log`.
2. Record from those misses:
   `cargo xtask record-fixtures --set base --from-log parity/reports/latest/<scenario>/<size>/python/fake-gh.log`.
   The `GET` misses are appended to the recipe, sorted and de-duplicated, so the recipe stays the source of truth and
   a re-recording is reproducible. Misses that aren't `GET`, or that are outside the recording scope, are listed at the
   end ("write these by hand", "refused") and not added.
3. Review the diff of `parity/fixtures/`. Look at what was kept, not only at what was scrubbed.
4. Commit the recipe and the fixtures together.

Use `--dry-run` to print the planned requests without running `gh`, and `--check` to record and compare with the
committed fixtures without writing anything (it exits 1 if any file differs). `--check` compares the exact bytes a
write would produce, so key order and formatting count, but it keeps the committed `note` (which holds the recording
date), so a re-recording on another day isn't drift.

Some recorded data depends on who records it: a repo object's `permissions` reflects the recorder's token, and
viewers with push or admin access also get keys such as `temp_clone_token`, the merge settings and
`security_and_analysis`. The recorder pins every `permissions` object to `pull` only and drops those keys (the list is
`VIEWER_ONLY_KEYS` in `xtask/src/fixtures/scrub.rs`), so a recording looks like what a signed-in user without push
access gets, whoever makes it. `check-fixtures` doesn't flag either, since hand-written fixtures such as `/user/repos`
model a user who owns their repos.

### The recipe

`parity/fixtures/<set>/recipe.toml` (or `--recipe`):

```toml
version = 1
allowed_repos = ["gizmo385/gh-lazy"]    # every /repos/<owner>/<name>/... path must be in this list
allowed_users = ["gizmo385"]            # /users/<login> and /users/<login>/repos are allowed for these

[[request]]
method = "GET"                          # only GET is accepted; this is the default
path = "/repos/gizmo385/gh-lazy/issues"
query = { state = "all", page = "1", per_page = "30" }
accept = "json"                         # a normalized accept name (see Key normalization); default "json"

[[request]]
path = "/repos/gizmo385/gh-lazy/readme"
accept = "raw"

[[request]]
path = "/repos/gizmo385/gh-lazy/actions/runs/123/logs"
binary = true                           # stored as a body_file and not text-scanned; must be opted in
```

Each request runs as `gh api -i -X GET -H "Accept: <media type>" "<path>?<canonical query>"`, up to 4 at a time.
The query goes in the path, so `-F` typing can't change values.

### Scope rules

Checked before any request is sent. A recipe entry that breaks one stops the run; a `--from-log` miss that breaks one
is reported as refused and not added to the recipe.

- The `gh` used (`--gh`, else the first `gh` on `PATH`) must not be fake-gh (`gh --version` contains `fake-gh`), and
  `gh auth status` must exit 0.
- Only `GET`. Write fixtures (merge, create issue, comment) are always hand-written.
- Allowed paths: `/repos/<owner>/<name>` and anything under it for repos in `allowed_repos`; `/users/<login>` and
  `/users/<login>/repos` for logins in `allowed_users`; `/emojis`; `/rate_limit`.
- Admin-only repo sub-paths are refused even under an allowed repo, because the recorder's token can read them but
  the public can't: `hooks`, `keys`, `invitations`, `collaborators`, `traffic`, `actions/secrets`,
  `actions/variables`, and `environments/<name>/secrets` and `environments/<name>/variables`.
- Everything else is refused. `/user*`, `/notifications*` and `/orgs/*` read the signed-in user's private scope, so
  they stay hand-written in `base` under the fictional login `parity-bot`.

### Scrub rules

Applied to every response before anything is written. An **abort** stops the run before any file is written, names
the request and the JSON pointer or byte offset, and exits 1. Secrets are never rewritten: a hit means the recipe is
wrong.

- **Headers**: only `Content-Type`, `Link` and `Location` are kept. Everything else (`Date`, `Etag`,
  `X-Ratelimit-*`, `X-Github-Request-Id`, `X-Oauth-*`, ...) is dropped.
- **`Link`**: GitHub writes page links at numeric-id paths, which fake-gh can't follow. Every URL (`next`, `prev`,
  `first`, `last`) at `/repositories/<id><rest>`, `/user/<id><rest>` or `/organizations/<id><rest>` is mapped back to
  the request's `/repos/<owner>/<repo><rest>`, `/users/<login><rest>` or `/orgs/<org><rest>`. Only the path changes;
  the host, the query (including `after=` cursors), the `rel`s and their order are kept byte for byte. A URL whose
  request path doesn't have the matching shape and the same `<rest>` is left as it is, and the post-write scrub check
  reports it. `Location` isn't touched, since a `/repositories/<id>` redirect there is real.
- **JSON bodies** (a JSON `Content-Type` and a body that parses), walked recursively:
  - `email`, `author_email` and `committer_email` string values become `user@example.com`. `null` stays `null`.
  - `avatar_url` becomes `https://avatars.githubusercontent.com/u/0?v=4`, and `gravatar_id` becomes `""`.
  - An object with `"private": true` aborts.
  - A string matching a secret pattern aborts. The patterns are GitHub tokens (`gh[pousr]_...`, `github_pat_...`),
    PEM private keys, AWS access key ids (`AKIA...`) and Slack tokens (`xox[baprs]-...`).
  - Everything else is kept, including timestamps (the app only shows absolute dates), `node_id`s, and other
    people's public logins and comment text.
- **Text bodies** (diffs, raw READMEs, logs): secret patterns abort. Emails become `user@example.com`, except
  addresses at `example.com` and `users.noreply.github.com`, SSH remotes (`git@github.com:o/r`), and image names
  whose "TLD" is an image extension (`logo@2x.png`; png, jpg, jpeg, gif, svg, webp). The text is otherwise kept byte
  for byte.
- **Binary bodies** (`binary = true` only) are stored without scanning. A body that isn't UTF-8 from an entry not
  marked binary aborts.

### What gets written

One fixture per request, at the canonical path and name for its key, in `parity/fixtures/<set>/`. JSON bodies go in
`body`, pretty-printed with 2-space indent and the response's key order. Text up to 64 KiB goes in `body_text`;
longer text and all binary bodies go in a `<name>.body.<ext>` sibling named by `body_file`. The `note` says
`recorded <YYYY-MM-DD> by cargo xtask record-fixtures`.

An existing fixture with the same key is overwritten. A hand-written one with the same key (and no `body` or `when`
matcher) under another name is renamed to the canonical name. Hand-written fixtures with no recipe entry are left
alone. If any file in the set fails to load, the run stops before sending a request, so it never writes over a file
it couldn't read. After writing, the run checks the set with the scrub check and `fake-gh lint`, and prints
`recorded N, refused M, write-by-hand K`.

### `cargo xtask check-fixtures`

Runs the same secret, email, avatar, `private`, header and numeric-id `Link` rules over every fixture under `parity/fixtures/`
(hand-written ones included), without network, and exits 1 on any finding: anything the scrubber would have
rewritten or refused. `body_file`s that aren't UTF-8 are treated as binary and skipped. `cargo test` runs it
(`xtask/src/fixtures/check.rs`), so CI enforces it.

## The parity harness (`cargo xtask parity`)

`cargo xtask parity` runs scenarios from `parity/scenarios/` against the Rust app, in a pseudo-terminal with an
isolated `HOME` and fake-gh on `PATH`, and dumps the screen at each capture point. The Python side comes from the
[recorded references](#recorded-references) in `parity/references/`, or from the Python app run the same way with
`--live-python`. See `docs/rust-rewrite/PLAN.md` §6.3. The code is in `xtask/src/parity/`.

### Running scenarios

```
cargo xtask parity [SCENARIO...] [--all] [--python-only | --rust-only] [--live-python[=SCENARIO,...]]
                   [--size COLSxROWS]... [--out DIR] [--keep-home] [--no-build] [--no-sync] [--python PATH]
                   [--retries N] [--jobs N] [--ci] [--validate]
cargo xtask parity [SCENARIO...] [--all] --live-python-since REV [--plan] [run flags as above]
cargo xtask parity --check-references [SCENARIO...] [--all] [--ci]
cargo xtask parity --record [--force] [SCENARIO...] [--all] [--jobs N] [--python PATH] [--no-sync] [--no-build]
                   [--out DIR] [--keep-home]
```

```sh
cargo xtask parity                         # every scenario: Rust live, Python from the references
cargo xtask parity startup --live-python   # one scenario, both apps live, Python checked against its reference
cargo xtask parity startup --python-only   # the Python side of one scenario, live
cargo xtask parity --validate              # check every scenario file, run nothing
```

- `SCENARIO` is a file stem from `parity/scenarios/` (`startup`) or a path to a `.toml` file. With none given,
  `--all` is implied.
- **Python from references (the default).** Before building or running anything, xtask checks the reference of every
  selected scenario that runs Python, at each size it runs (as `--check-references` does; a `--size` the scenario
  doesn't list has no reference and counts as missing). If any is stale, missing or invalid, it prints the
  `--check-references` line with ` (or use --live-python)` appended, plus the `::error` annotations with `--ci`, and
  exits `2`:
  `stale reference: startup/160x48: scenario changed; run cargo xtask parity --record startup (or use --live-python)`.
  Otherwise the reference is copied into the report as the Python run's dumps (`startup 160x48 python: recorded`) and
  only Rust runs. No Python process starts, so this needs no `uv sync` and no `.venv`.
- **Python live.** `--live-python` runs Python live for every selected scenario, and `--live-python=a,b` for those
  (naming a scenario that isn't selected is an error, exit `2`; the others read their references). `--python-only`
  runs it live too. After each live Python run whose reference is fresh, the run is compared with the reference by
  the rule recording uses ([Recording](#recording)); where they disagree, that's **drift**:
  `startup 160x48 main: drift: rows [[1] Repositories]: table 0 row 2 differs; the reference is wrong or
  nondeterministic: run cargo xtask parity --record --force startup and review the diff`. Drift goes into the
  capture's `check.json` (`drift`), counts against Python, so the scenario fails whatever its `gate`, and is an
  `::error` with `--ci`. A disagreement about the served requests is listed on the first capture, as a `requests`
  mismatch. A stale, missing or invalid reference skips the check with a note
  (`startup 160x48: reference stale, drift not checked`), which isn't an error, so a PR can be checked live before
  its references are re-recorded. What the live run gets wrong on its own (a failure, its `[expect]`) is reported as
  usual, not as drift.
- **Python live where its inputs changed (CI).** `--live-python-since REV` compares `REV` with the working tree
  (`git diff --name-only --no-renames REV`, plus untracked files that aren't ignored). If any changed path is under
  the Python app or harness inputs, the Python-backed tests, `.github/workflows/rust.yaml`, a `generator`, `golden`
  or `test` path of a [golden manifest](#manifest-format-v1), or a `vt100`/`portable-pty` version in `Cargo.lock`
  changed, every selected scenario runs Python live, as with `--live-python`. Otherwise only the selected scenarios
  whose `parity/references/<name>/` changed run it live, as with `--live-python=a,b`, and the rest read their
  references; references of a scenario that's deleted, not selected or doesn't run Python are ignored. The run
  prints the plan first (`parity: live-python=startup (since HEAD^1)`), and drift is checked as above. With
  `--plan` it prints only the plan line, `live-python=all`, `live-python=none` or `live-python=a,b,c`, and exits
  `0` without building or running anything. The path lists are in `xtask/src/parity/reference.rs`
  (`PYTHON_APP_INPUTS`, `HARNESS_INPUTS`, `CI_PYTHON_PATHS`), the same lists that decide whether a reference is
  fresh.
- `--rust-only` neither reads nor checks references.
- Before running, xtask builds `lazy-github` and `fake-gh` (`cargo build -p lazy-github -p fake-gh`) unless
  `--no-build`. When a selected scenario runs Python live, it also runs `uv sync --frozen --quiet` under your real
  environment unless `--no-sync`. uv is never run under the isolated `HOME`, where it would start from an empty cache.
- Each scenario runs once per size and per app, in a fresh isolated environment. Sizes and apps within a scenario run
  one after another.
- The report goes to `parity/reports/latest/` (wiped first, and git-ignored) or to `--out DIR`. With `--out`, only the
  directories of the scenarios being run are replaced.

| Flag | Meaning |
| --- | --- |
| `--all` | Run every scenario in `parity/scenarios/`. |
| `--python-only`, `--rust-only` | Run one app only: for capturing references and for debugging. `--python-only` runs Python live and checks it for drift; `--rust-only` doesn't read references. |
| `--live-python[=SCENARIO,...]` | Run Python live instead of reading its reference: in every selected scenario, or in those named (after `=`, comma-separated). Fresh references are checked for drift. Conflicts with `--rust-only`, `--record` and `--check-references`. |
| `--live-python-since REV` | Run Python live in every selected scenario if a Python input changed since `REV`, otherwise in those whose references changed (above). Conflicts with `--live-python`, `--python-only`, `--rust-only`, `--record` and `--check-references`. |
| `--plan` | With `--live-python-since`: print the plan line and exit `0`, building and running nothing. |
| `--size COLSxROWS` | Run at this size instead of the scenarios' `sizes`. Repeatable. |
| `--out DIR` | Write the report here instead of `parity/reports/latest/`. |
| `--keep-home` | Copy each run's `home/` and `work/` (and `remotes/`, with `git_remote_branches`) into its report directory. |
| `--no-build` | Skip `cargo build -p lazy-github -p fake-gh`. The binaries must already be in `target/<profile>/`. |
| `--no-sync` | Skip `uv sync --frozen` (which only runs when Python runs live). |
| `--python PATH` | Run `PATH -m lazy_github` instead of the repo's `.venv/bin/python` (`.venv\Scripts\python.exe` on Windows). |
| `--retries N` | Re-run a failed app run up to N times. A run that passes on retry is marked `flaky: true`. Default 0, or 1 with `--ci`. A Rust run in a scenario with `gate = false` is never retried, since it can't fail the gate. |
| `--jobs N` | Run up to N scenarios in parallel. Default: the CPU count, at most 4. More runs at once slow each app enough to risk timeouts. |
| `--ci` | Running in CI: `--retries 1` unless `--retries` is given, each condition `wait` capped at 3 s in runs that can't fail the gate (the Rust side of a `gate = false` scenario, which usually times out waiting for something it can't draw yet; stability waits aren't capped), `--color never` for the nested `cargo build` and `uv sync`, and GitHub Actions annotations at the end of the output (see [CI](#ci)). |
| `--validate` | Parse and validate the scenario files (all, or the ones named) and exit. |
| `--check-references` | Check the [recorded references](#recorded-references) of the scenarios (all, or the ones named) against the current inputs and exit: `0` when all are fresh, `1` otherwise. Builds and runs nothing. Takes only scenario names, `--all` and `--ci`. |
| `--record` | [Record](#recording) the stale and missing references of the scenarios (all, or the ones named) by running Python, and delete orphans. Builds only `fake-gh`. Run dumps go to `--out`, default `parity/reports/record/` (wiped first). Conflicts with `--python-only`, `--rust-only`, `--live-python`, `--size`, `--retries`, `--ci`, `--validate` and `--check-references`. |
| `--force` | With `--record`: re-record fresh references too. |

Exit codes:

- `0`: every gated scenario passed and the Python side is clean everywhere.
- `1`: a gated scenario failed, or the Python side failed anywhere (timeout, crash, fixture miss, failed
  `[expect]`, drift).
- `2`: a scenario file is invalid (printed as `file:line: message`), a reference the run reads isn't fresh, or the
  harness itself failed.

Rust-side failures in scenarios with `gate = false` are reported as `rust-not-ready` and don't change the exit code,
so scenarios can land before the Rust UI exists. Until M1 the Rust binary just exits, so every scenario's Rust side
is `rust-not-ready`.

### The isolated environment

Every scenario × size × app gets a temp dir `<run>` holding `home/`, `bin/` and `work/` (and `remotes/` with
`git_remote_branches`). It's removed after the run; `--keep-home` copies `home/`, `work/` and `remotes/` into the
report first.

- `home/.config/lazy-github/config.json` is built from three layers, in order:
  - the scenario's `config` file, `parity/configs/<name>.json` (default `default`);
  - deep-merged with the scenario's `config_overrides` (objects merge key by key; anything else replaces);
  - then always forced: `core.first_start = false` and `api.client_type = "GITHUB_CLI"`, which keep the Python app
    off its first-start screen and on the `gh` backend.

  The Python app keeps everything (config, log, table cache, PR drafts) under `$HOME/.config/lazy-github`, so a fresh
  `HOME` also means no cached tables are shown before the first fetch.
- `bin/gh` (`gh.exe` on Windows) is fake-gh and `bin/fake-ssh` is [fake-ssh](#fake-ssh), installed with
  `fake_gh::install`.
- With `seed = "<name>"`, `parity/seeds/<name>/home/` is copied into `home/` and `work/` into `work/` before the app
  starts: after `config.json` is written and after the git repo below is made. Copying after `git init` keeps the
  parity commit empty, so its SHA doesn't change, and seeded `work/` files are untracked. Each app run gets a fresh
  copy, so a file one run rewrites never reaches the next. A seed is rejected (with `file:line`, by `--validate` and
  every run) when its name isn't `[a-z0-9][a-z0-9-]*`, its directory is missing, it holds anything at the top level
  other than `home/` and `work/`, it holds no files, it holds a symlink anywhere, or it holds
  `home/.config/lazy-github/config.json`, which the harness owns (use `config` and `config_overrides`).
- `work/` is the app's working directory. When the scenario sets `git_remote = "owner/name"`, it's made a git repo:
  - `git init -q -b <git_branch, default main>`
  - `git remote add origin https://github.com/owner/name.git`, or `git@github.com:owner/name.git` with
    `git_remote_protocol = "ssh"`. Python keeps the `.git` of an HTTPS remote in the repo name (Py bug B5), so it
    never finds that repo; a scenario where both apps should pick up the current directory's repo uses `"ssh"`.
  - `git commit -q --allow-empty -m parity` as `parity-bot <parity-bot@example.com>` dated `2026-01-01T00:00:00Z`,
    so the commit SHA is the same on every run.
- With `git_remote_branches = ["main", ...]` (which needs `git_remote_protocol = "ssh"`), `origin` reaches a local
  repo. `origin` stays `git@github.com:owner/name.git`: both apps derive the current directory's repo from
  `git remote get-url origin`, so a local path or `url.<base>.insteadOf` (which `get-url` expands) would lose it.
  Right after the commit above, with the same environment:
  - `git init -q --bare -b <first listed branch>` makes `<run>/remotes/<owner>/<name>.git` (`-b <git_branch>` when
    the list is empty, which leaves it with no branches);
  - `git push -q <bare repo path> HEAD:refs/heads/<branch>` pushes the commit as each listed branch. It pushes by
    path, so the work branch keeps no upstream;
  - the app gets `GIT_SSH=<run>/bin/fake-ssh`, `GIT_SSH_VARIANT=simple` and `FAKE_SSH_ROOT=<run>/remotes`, so
    `git ls-remote origin <b>` and `git push origin ...` in `work/` go through [fake-ssh](#fake-ssh) to that repo.

  Without the key there's no `remotes/` and none of the three variables.
- The environment is cleared, then only these are set:
  - `HOME` (plus `USERPROFILE` on Windows), and `XDG_CONFIG_HOME`/`XDG_CACHE_HOME`/`XDG_DATA_HOME`/`XDG_STATE_HOME`
    under `home/`
  - `PATH=<run>/bin` followed by the inherited `PATH`
  - `TERM=xterm-256color`, `COLORTERM=truecolor`
  - `LANG=C.UTF-8` on Linux, `LANG=en_US.UTF-8` on macOS
  - `TZ=UTC`. No fixed clock is needed: the Python UI renders only absolute timestamps.
  - `LAZY_GITHUB_ORIGINAL_PWD=<run>/work`
  - `FAKE_GH_FIXTURES` (the scenario's sets, resolved under `parity/fixtures/`), `FAKE_GH_LOG=<report dir>/fake-gh.log`
    and `FAKE_GH_AUTH`
  - `TEXTUAL_ANIMATIONS=none`, `PYTHONDONTWRITEBYTECODE=1`
  - `GIT_CONFIG_NOSYSTEM=1`
  - `TMPDIR=<run>/tmp` on macOS

  `GH_TOKEN`, `GITHUB_TOKEN`, `GH_HOST`, `NO_COLOR`, `TEXTUAL` and everything else are never passed through.
- Commands: Python runs `<repo>/.venv/bin/python -m lazy_github` (or `--python`), and Rust runs
  `target/<profile>/lazy-github`. Neither gets arguments.

### Scenario format (v1)

A scenario is `parity/scenarios/<name>.toml`; the file stem is its name. The name must match `[a-z0-9][a-z0-9-]*`,
since it names the scenario's report directory, and names must be unique within a run.

```toml
version = 1
description = "Main screen right after launch, no current-directory repo"
gate = false                     # true = a Python/Rust structural mismatch fails CI (#70). Feature PRs flip it.
apps = ["python", "rust"]        # default both
parity_rows = ["main-layout.layout"]
                                 # parity matrix row ids (docs/rust-rewrite/parity/areas/); checked by
                                 # `cargo xtask parity-matrix`, shown in the report
sizes = [[160, 48], [100, 30]]   # [cols, rows]; default [[160, 48]]
timeout_ms = 30000               # whole run per app per size; default 30000
key_delay_ms = 30                # pause after each key; default 30 (after "escape" always >= 100)
capture_stable_ms = 300          # implicit stability wait before each capture; default 300

[env]
fixtures = ["base"]              # sets under parity/fixtures/, earlier overrides later; default ["base"]
config = "default"               # parity/configs/<name>.json; default "default"
gh_auth = "logged-in"            # or "logged-out"; default "logged-in"
git_remote = "gizmo385/gh-lazy"  # optional; omit = CWD is not a git repo
git_remote_protocol = "https"    # or "ssh" (origin is git@github.com:...); default "https"; needs git_remote
git_branch = "main"              # optional, default "main"
git_remote_branches = ["main"]   # optional: a local remote for origin with these branches (see The isolated
                                 # environment); needs git_remote_protocol = "ssh"
seed = "repos-cache"             # optional: parity/seeds/<name>/, copied into home/ and work/ (see Seeds)

[env.config_overrides]           # optional, deep-merged over the config file
appearance = { show_command_log = true }

[[step]]
wait = { text = "Repositories", timeout_ms = 10000 }

[[step]]
capture = "main"

[[step]]
keys = ["j", "j", "enter"]

[[step]]
wait = { stable_ms = 500 }

[[step]]
type = "gh-lazy"                 # literal text, sent as UTF-8 keystrokes

[[step]]
click = { text = "O Lookup Repository" }   # left click on the first cell of the first match

[[step]]
wheel = { at = [20, 9], direction = "down" }   # one wheel notch over a 0-based [col, row]

[[step]]
move = { at = [100, 1] }         # motion only: park the pointer

[[step]]
capture = "after-select"
checks = ["titles", "tabs", "headers", "rows", "cursor", "footer", "focus"]   # default: all of these
ignore = { boxes = ["Details"] } # skip the listed checks inside boxes whose title contains this text

[expect.main]                    # optional, checked against EACH app on its own (#93)
pane_titles = ["", "[1] Repositories", "[2] Pull Requests (empty)"]
focused = ["[1] Repositories"]
contains = ["Repositories"]
not_contains = ["Traceback"]
footer = ["q Quit", "^p Command Pallete"]
tabs = { box = "Details", names = ["Info"], active = "Info" }
```

Rules. The parser rejects anything else, including unknown keys, with exit 2 and a `file:line` message:

- Every `[[step]]` table has **exactly one** of these keys:
  - `wait`, a table with exactly one of:
    - `text = "<substring>"`: appears on any row
    - `gone = "<substring>"`: no longer on any row
    - `regex = "<pattern>"`: matched against the screen with rows joined by `\n` (Rust `regex` syntax)
    - `stable_ms = <n>`: the screen's characters are unchanged for n ms. Colors and the cursor are ignored, so a
      blinking cursor doesn't reset the timer.
    - `log = { method = "PUT", path = "/repos/o/r/pulls/1/merge" }`: the fake-gh log has a served request matching
      it. `query`, `accept` and `body` can be given too, as in a fixture's `when.after` entry.
    - `remote_branch = "<name>"`: the run's local remote has `refs/heads/<name>` (checked with
      `git --git-dir <bare repo> rev-parse --verify --quiet`), so a scenario can assert that a push happened. Only in
      a scenario with `git_remote_branches`.

    Each `wait` takes an optional `timeout_ms` (default 10000). A timeout fails that app's run at that step and dumps
    the screen.
  - `keys`: a list of key specs, below.
  - `type`: a string sent verbatim.
  - `paste`: a string. It's sent in bracketed-paste mode (`ESC[200~ … ESC[201~`) if the app has enabled it, else
    verbatim.
  - `capture`: a name. It must be unique in the scenario and match `[a-z0-9-]+`. An implicit
    `wait.stable_ms = capture_stable_ms` runs first. `checks` and `ignore` are allowed alongside it, and only there.
  - `sleep_ms`: a number.
  - `resize = [cols, rows]`: resizes the PTY and the parser.
  - `click`, `wheel` and `move`: mouse steps, below.
- **Mouse steps** send SGR 1006 mouse reports. Each is a table with exactly one target:
  - `at = [col, row]`: a cell, 0-based like `grid.json`, `structure.json` rects and `failure.txt`.
  - `text = "<substring>"`, with optional `nth` (1-based, default 1), `offset = [dcol, drow]` (default `[0, 0]`)
    and `timeout_ms` (default 10000), which are rejected without `text`. The match runs on each row's text as
    `<capture>.txt` has it (wide-character continuation cells skipped) and maps back to the cell; matches are counted
    row by row, top to bottom, left to right, non-overlapping. The target is the first cell of the `nth` match plus
    `offset`. Like `wait.text`, the step polls until the match is on screen and times out if it never is, so it needs
    no `wait` before it. Matching is per app, on its own screen, so text targets keep working when the apps place the
    same text a cell apart. Use `at` for blank areas (parking the pointer, a pane's interior for the wheel).

  The steps:
  - `click`: a motion report to the target, then a left press and release, in one write, then `key_delay_ms`. Two
    clicks on one cell within 0.5 s are a double click to Textual, so put a capture or a wait between them.
  - `wheel`: `direction` is `up`, `down`, `left` or `right` (required) and `count` is 1 or more (default 1). A motion
    report, then `count` wheel reports with `key_delay_ms` after each.
  - `move`: one motion report, then `key_delay_ms`.

  | Event | Bytes (`x = col + 1`, `y = row + 1`) |
  | --- | --- |
  | motion, no button | `CSI < 35 ; x ; y M` |
  | left press / release | `CSI < 0 ; x ; y M` / `CSI < 0 ; x ; y m` |
  | wheel up / down / left / right | `CSI < 64` / `65` / `66` / `67` `; x ; y M` (no release, as terminals send) |

  When the step runs, the app must have turned on mouse reporting with SGR encoding, and the target must be on
  screen; otherwise the run fails at that step as `step-failed`, with the reason in `status.json`'s `message`
  (`click: the app hasn't enabled SGR mouse reporting (mode None, encoding Default)`,
  `click: target [161, 3] is off the 160x48 screen`). Motion reports are sent only in any-motion mode (`?1003h`), as
  a terminal does: in another mode `click` and `wheel` skip theirs and `move` fails. Both apps use any-motion mode.

  Keys stay the way to drive a flow. Use the mouse only for the behavior under test, so a scenario doesn't depend on
  hit-testing it isn't about. Don't send mouse reports with `type`: it hard-codes 1-based cells, can't follow text and
  doesn't check that the app turned mouse reporting on.
- **Key specs** use Textual's binding spelling from `lib/bindings.py`, so defaults can be copied. They're encoded as
  xterm sequences:

  | Spec | Bytes |
  | --- | --- |
  | one printable character (`"j"`, `"J"`, `"/"`, `"?"`) | its UTF-8 |
  | `enter` | `\r` |
  | `tab` | `\t` |
  | `shift+tab` | `ESC[Z` |
  | `escape`, `esc` | `ESC` |
  | `backspace` | `0x7f` |
  | `space` | ` ` |
  | `up` `down` `right` `left` | `ESC[A` `ESC[B` `ESC[C` `ESC[D`, or `ESCOA` … when the app enabled DECCKM |
  | `home` `end` | `ESC[H` `ESC[F`, or `ESCOH` `ESCOF` with DECCKM |
  | `insert` `delete` `pageup` `pagedown` | `ESC[2~` `ESC[3~` `ESC[5~` `ESC[6~` |
  | `f1`–`f4` | `ESCOP` `ESCOQ` `ESCOR` `ESCOS` |
  | `f5`–`f12` | `ESC[15~` `ESC[17~` `ESC[18~` `ESC[19~` `ESC[20~` `ESC[21~` `ESC[23~` `ESC[24~` |
  | `ctrl+a`…`ctrl+z` | `0x01`…`0x1a` |
  | `alt+<char>` | `ESC` + the char |

  `ctrl+m` and `shift+enter` are rejected: terminals can't send them distinctly without the kitty protocol, and that's
  a keymap design question (parity matrix, `docs/rust-rewrite/parity/areas/terminal-caveats.md`).
- `[expect.<capture>]` keys must name a capture in the scenario. All of them are optional. `pane_titles` and `focused`
  are exact ordered lists. `footer` is an exact ordered list of `"<key> <label>"`.
- `checks`, `ignore` and `[expect]` are evaluated by the structural check, below.

### Keeping the suite fast

Every scenario runs in CI on every PR, once per size and per app, and Actions minutes are the limit. Startup
dominates a run (about 2.5 s for Python and 0.4 s for Rust), so the number of runs matters most. Before adding a
scenario:

- **Add captures, not scenarios.** If an existing scenario already has the env your rows need (fixtures, config,
  `gh_auth`, `git_remote`), add steps and captures to it and list the new rows in its `parity_rows`. The report and
  the checks are per capture, so a failure still names the exact screen. A new scenario is for a new env, or for a
  flow whose earlier steps would change the screen your rows are about.
- **One size by default.** Add `[100, 30]` (or another size) only for rows about layout, clipping or wrapping: pane
  sizes, a footer or header cut at the edge, text that wraps. Focus, content, counts and key dispatch don't change
  with the size.
- **Wait on a signal.** Before a capture, wait for `text`, `gone`, `regex` or `log` on the thing the capture is
  about. Don't add `stable_ms` waits or `sleep_ms` to cover a slow redraw: the capture's own `capture_stable_ms`
  wait already runs, and a concrete wait ends as soon as the screen is ready. A `gone` wait also beats `sleep_ms`
  for a timer (`gone = "View Maximized"`, not `sleep_ms = 5500`).
- **One timer per behavior.** Checks that wait out a timer (a toast expiring, a refresh interval) cost their whole
  duration in every run. Check each timer in one scenario at one size, and keep other scenarios from waiting for it.
- **Park the pointer after a click.** A footer item under the pointer stays hovered (in both apps), and a hovered
  item's styles change how the extractor reads the footer. Before a capture after a click, `move` the pointer to a
  blank cell (the status box), as a real pointer moving away would.
- **Re-record only what you changed.** `cargo xtask parity --record <scenario>` records just that scenario's
  [references](#recording), and skips fresh ones. `--record --force` with no names runs Python at least once for
  every reference, and twice where the screen changed (about 30 s to a minute today with 4 jobs); a harness or
  Python-app change makes every reference stale anyway, so a plain `--record` does the same then.

### How a run works

- The app runs in a `portable-pty` pseudo-terminal at the scenario's size. A reader thread feeds its output into a
  `vt100` parser and appends the raw bytes to `pty.raw`.
- Input (keys, text, pastes and query replies) is queued to a writer thread that owns the PTY's input side, so
  neither the step loop nor the reader ever blocks on a write the app isn't reading, and `timeout_ms` always fires.
- The reader answers the terminal queries apps wait on, which `vt100` doesn't: cursor position (`ESC[6n`, from the
  parser's cursor), primary and secondary device attributes (`ESC[c` → `ESC[?62;22c`, `ESC[>c` → `ESC[>0;10;1c`) and
  DECRQM (`ESC[?<n>$p` → `ESC[?<n>;0$y`, "not recognized"). The kitty keyboard query (`ESC[?u`) is deliberately left
  unanswered: crossterm sends DA1 right after it, and the DA1 reply tells it there's no kitty support without waiting
  out its 2 s timeout.
- Mouse steps' reports go through the same writer thread. Whether they're sent depends on the mouse mode the app
  turned on, which the runner reads from the parser (Textual writes `?1000h ?1003h ?1015h ?1006h`, crossterm adds
  `?1002h`; `vt100` ignores `?1015h`). The harness answers the in-band resize query (`?2048$p`) as not recognized,
  so Textual leaves SGR pixel mode (`?1016h`) off and coordinates are cells.
- Steps run in order. Waits poll the screen every 20 ms. Each key is followed by `key_delay_ms` (at least 100 ms after
  `escape`, so Textual's escape-sequence timeout doesn't fuse it with the next key).
- The whole run is bounded by `timeout_ms`. On a step timeout, a failed step, a crash, or an exit before the last
  step, the runner writes `failure.txt` and `failure.grid.json` (the screen at that moment) and records the status.
- After the last step the app is killed and given 2 s to exit.

### Report layout

```
summary.json                                     # all results
<scenario>/<cols>x<rows>/<app>/status.json       # how the run ended, fixture misses, duration
<scenario>/<cols>x<rows>/<app>/fake-gh.log       # the request log
<scenario>/<cols>x<rows>/<app>/pty.raw           # raw PTY output, for debugging
<scenario>/<cols>x<rows>/<app>/<capture>.txt     # plain text screen
<scenario>/<cols>x<rows>/<app>/<capture>.grid.json
<scenario>/<cols>x<rows>/<app>/failure.txt       # only when the run failed (plus failure.grid.json)
<scenario>/<cols>x<rows>/<app>/home/, work/      # only with --keep-home (remotes/ too, with git_remote_branches)
<scenario>/<cols>x<rows>/<app>/<capture>.structure.json   # what the extractor recognized
<scenario>/<cols>x<rows>/<capture>.check.json    # structural mismatches, and drift
```

When Python comes from a reference, its directory holds what the reference stores, in the same form: each
`<capture>.grid.json` and `<capture>.txt` (from the `.screen`), `fake-gh.log` (the normalized `requests.jsonl`) and a
`status.json` with `"recorded": true`. Nothing ran, so there's no `pty.raw` and no `failure.txt`.

The report (#94) adds
`<capture>.diff.txt`, `report.html`, `index.html` and `summary.md`.

### Screen dumps

`<capture>.txt` holds one line per screen row: the cells' contents, with wide-character continuation cells skipped and
trailing spaces trimmed.

`<capture>.grid.json` holds the full grid with run-length styles (`xtask/src/parity/grid.rs` owns the type):

```json
{"version":1,"cols":160,"rows":48,"cursor":{"row":0,"col":0,"visible":false},
 "lines":[{"cells":["┌","─","─"," ","[","1","]"],"runs":[{"col":0,"len":20,"fg":"#4ebf71","bg":"#1e1e1e","bold":false,"italic":false,"underline":false,"inverse":false}]}]}
```

- `cells` has one entry per column. A blank cell is `" "`, and the continuation cell after a wide character is `""`,
  so column indexes stay exact.
- `runs` cover the whole row, left to right; each is a run of consecutive columns with identical style.
- Colors are `"#rrggbb"` for truecolor, `"idx:<n>"` for an indexed color and `null` for the terminal default.
- `cursor` is 0-based.

### Status and results

Each app run writes `status.json`:

```json
{"status": "ok", "step": null, "exit_code": null, "duration_ms": 2400, "flaky": false, "fixture_problems": false,
 "misses": [], "errors": []}
```

- `status` is `ok` (every step ran), `timeout` (a step or the whole run took too long), `exited` (the app exited
  before the last step), `step-failed` (a step couldn't run against the app as it was, e.g. a mouse step before the
  app turned on mouse reporting; `message` says why) or `spawn-error` (the PTY or the app couldn't be started;
  `message` says why).
- `step` is the index of the failing step, and `exit_code` the app's exit code when it exited.
- `misses` are the fake-gh log lines with `"miss": true`, ready for `cargo xtask record-fixtures --from-log` (#68), and
  `errors` the lines with an `error`. Either makes `fixture_problems: true`, and the run counts as failed even though
  its status is `ok`.
- `recorded: true` marks a Python side read from its [recorded reference](#recorded-references): status `ok`,
  `duration_ms: 0`, and the console, the index and the run table say `recorded` instead of a duration. The field is
  left out when false, so a live run's status is unchanged.

`summary.json` collects everything:

```json
{"version": 1, "scenarios": [{"name": "startup", "description": "…", "gate": false, "parity_rows": ["…"],
  "result": "rust-not-ready",
  "sizes": [{"size": "160x48", "apps": {"python": <status>, "rust": <status>},
             "captures": [{"name": "main", "check": "pass", "mismatches": 0, "against": []}]}]}]}
```

An app that didn't run is left out of `apps`. A scenario's `result` is:

- `pass`: both apps ran and passed, and every check passed.
- `fail`: the Python side failed (including its `[expect]`), or a gated scenario's Rust side did (including a
  mismatch). `error` is set when the harness itself failed.
- `rust-not-ready`: not gated, and the Rust side failed or mismatched.
- `python-only` / `rust-only`: only one app ran (a flag or the scenario's `apps`), and it passed.

Each capture in `captures` carries its structural check:

```json
{"name": "main", "check": "fail", "mismatches": 1, "against": ["python"]}
```

- `check` is `pass`, `fail`, or `n/a` when there was nothing to check (no `[expect]` for the capture and no cross-app
  comparison, for example because only one app produced the capture).
- `against` lists the apps the mismatches count against: an app whose `[expect]` failed, `rust` for any cross-app
  mismatch, and `python` for drift. An app with a mismatch counts as failed in the result rules above, so a Python
  `[expect]` failure or drift always fails the run, and a Rust mismatch makes an ungated scenario `rust-not-ready`.
  `mismatches` counts drift too.

`<capture>.check.json` holds the mismatches: `expect` (per app), `cross` (Python against Rust) and, after a live
Python run with a fresh reference, `drift` (left out when empty). A drift mismatch has the same keys as a cross-app
one, with the live run's value under `python` and the reference's under `rust`, and its `message` says so.

### Structure extraction

After each size, every capture's `<capture>.grid.json` is turned into `<capture>.structure.json`
(`xtask/src/parity/structure.rs`, `extract`). Extraction is pure and runs as passes over the grid, each claiming the
cells it recognized so later passes skip them: boxes, nesting, the footer, then inside each box tabs, tables and
text.

```json
{"version": 1, "size": [160, 48],
 "boxes": [{"id": 1, "parent": null, "title": "[1] Repositories", "subtitle": "", "rect": [0, 3, 64, 17],
            "border_fg": "#4ebf71", "focused": true, "tabs": null,
            "tables": [{"headers": ["★", "Owner", "Name", "Private"], "headers_clipped": false,
                        "rows": [["☆", "parity-bot", "dotfiles", "✘"]], "cursor": 0}],
            "text": []}],
 "tabs": null, "footer": [{"key": "q", "label": "Quit"}], "footer_clipped": true,
 "focused": ["[1] Repositories"], "loose_text": []}
```

`xtask/src/parity/snapshots/` holds the full structure of the Python `startup` screen at 160x48 and 100x30, extracted
from the dumps in `xtask/tests/grids/python/`.

#### Boxes

- **Families.** Each family is a set of corners, a horizontal char and a vertical char:

  | Family | Corners (┌ ┐ └ ┘ order) | Horizontal | Vertical |
  | --- | --- | --- | --- |
  | light | `┌┐└┘` | `─` | `│` |
  | rounded | `╭╮╰╯` | `─` | `│` |
  | heavy | `┏┓┗┛` | `━` | `┃` |
  | double | `╔╗╚╝` | `═` | `║` |
  | ascii | `+` | `-` | `\|` |

  Textual's `solid` border, which every Python pane uses, is the light family.
- **Tracing**, from every top-left corner:
  - Scan right over horizontal chars and title text to the first top-right corner. That corner must have the
    top-left's fg, or there's no box. Any other box-drawing char on the way also means there's no box.
  - Scan down both sides over the family's vertical char, in the same fg, to the bottom corners (same fg).
  - The bottom edge is horizontal chars or subtitle text.
  - Corners of the other box-drawing families are accepted when the fg matches, as when borders are joined. ascii
    boxes only close with `+`, need both sides made entirely of `|`, and must be at least 3×3, so `C++` or `a + b` in
    text isn't a box.
  - `rect` is `[col, row, width, height]` of the outer border, and `border_fg` is the top-left corner's fg.
- **Title** is the top edge between the corners. Runs of border chars split it into segments, which are trimmed, and
  empty ones are dropped; `title` is the segments joined with `" | "` (`─ A ─── B ─` gives `A | B`). In an ascii box a
  single `-` inside text (`my-repo`) doesn't split. **Subtitle** is the bottom edge, read the same way. An untitled
  box has `""`.
- **Nesting.** A box entirely inside another's interior gets `parent` set to the smallest such box. Boxes are listed
  in pre-order: top-level boxes by (row, col), each followed by its children by (row, col). `id` is the index in that
  order. On the main screen that gives the status summary, Repositories, Details, Pull Requests, Issues, Workflows.
- **Limits.** Textual `thick`, `tall`, `wide` and `blank` borders, drawn with half blocks or spaces, aren't boxes in
  v1. Most Python modals use `thick`; their content lands in `loose_text` and is still searched by `contains`. The
  first modal spec (M3) extends the extractor.

#### Focus

- The focus color comes from `parity/themes.toml`, keyed by the scenario config's `appearance.theme` (default
  `textual-dark`, the Python default). It's Textual's `$success`, measured from the Python startup dump: `#4ebf71` for
  `textual-dark`. A value is `#rrggbb` or, for the ANSI themes, which draw in the terminal's palette, an indexed
  color as the grid dumps it (`idx:2`).
- A box is `focused` when its `border_fg` is the focus color, and the top-level `focused` lists the focused boxes'
  titles in pre-order.
- **Limits.** A theme whose focus color also draws an unfocused border gives false positives. A theme missing from
  `themes.toml` gives `focused = []` and a `warnings` entry, which is also printed. Measured so far: `textual-dark`, and
  for the `theme-*` scenarios `catppuccin-latte`, `ansi-dark` and `no-such-theme` (an unknown name, which falls back to
  `textual-dark`). `textual-light`'s `$success` is `textual-dark`'s, so a scenario can't tell those two apart by focus.

#### Inside a box

Each pass looks at the box's interior minus nested boxes and claimed cells. First, **scrollbar columns** are dropped
from the right edge: a column whose background (with `inverse` applied) differs from its row's most common background
on at least half the rows, and that holds only blanks and `▁▂▃▄▅▆▇█▉▊▋▌▍▎▏▐▔▕` where it differs. Textual's vertical
scrollbar is two such columns. Scrollbar glyphs at the end of a row (a horizontal scrollbar's thumb) are ignored too.

- **Tabs.**
  - A row is a tab bar when its non-blank cells are all in `━╸╺─▔▁`, span at least 50% of the interior width, and
    use at least two fg colors. Textual draws `╸━━━╺━━━` with the active tab's segment in another color.
  - The row directly above must have text. The tab names are its segments separated by two or more blanks.
  - The bar cells whose fg differs from the bar's most common fg are the active segment. The active tab is the name
    overlapping it most (`null` if none does).
  - The box gets `"tabs": {"names": [...], "active": "..."}`, and both rows are claimed. The area outside every box
    is searched the same way, into the top-level `tabs`.
  - **Limit:** a tab label containing two consecutive blanks is split in two.
- **Tables.**
  - A **header row** is a row whose text cells are all bold and share one background other than the box's (its
    border's background), when neither the row above nor the row below has that same style.
  - The header's segments (separated by two or more blanks) are `headers`, and their start columns are the column
    boundaries. `headers_clipped` is true when the last header ends within one cell of the interior's right edge,
    so the screen may have cut it off (Python's Workflows at 100x30 shows `Run` for `Run #`).
  - Each following row is split at the boundaries and its cells trimmed, giving `rows`. The table ends at an empty
    row, at a row in the header's style (a stacked table) or at the end of the interior. A box can hold several
    tables.
  - **Cursor.** `cursor` is the index of the single row whose background differs from the other rows' common one.
    With fewer than three rows there's no majority, so the single bold row (Textual's cursor style) is the cursor.
    Otherwise it's `null`.
  - **Limits:** a header cell with two consecutive blanks is split. A table without a styled header isn't detected,
    and its rows land in `text`. A single bold line on its own background (a one-line bold code block, say) is read as
    a table with no rows.
- **Text.** `text` is the box's remaining interior, row by row: each run of unclaimed cells, trimmed, with empty ones
  dropped. `loose_text` is the same for the area outside every box and the footer. Neither is compared across apps,
  because Markdown and wrapping legitimately differ. They're there for the report.

#### Footer

- The last screen row is the footer unless a box covers it. Otherwise the footer is the last inner row of the
  innermost box over the last screen row, as on screens that put `LazyGithubFooter` inside a bordered container (the
  auth screen, settings, the lookup and confirm modals). That row only counts when every binding read from it has a
  label, so ordinary box content isn't taken for a footer. The row is split into style runs, and the style of the
  first run with text is the key style.
- Each key-style run with text starts a binding: its trimmed text is `key`, and the trimmed text of the runs up to
  the next key run is `label`.
- An empty last row gives `footer = []`. `footer_clipped` is true when the row's last cell isn't blank, which means
  the screen may have cut the footer off.

#### Changes from the v1 rules in #93

The rules were checked against the Python `startup` dumps at 160x48 and 100x30, and adjusted where Textual's output
differs:

- **Header rows need bold and a background.** The v1 rule (bold *or* a background) read the bold Markdown headings
  in the Details pane as one-column tables. Textual's `DataTable` header is both bold and on its own background. The
  reference background is the box's border background, not the most common interior one, because a box full of rows
  makes the row background the most common. A header also mustn't share its style with the row above, so the last
  line of a highlighted block isn't a header.
- **Backgrounds account for `inverse`.** Textual draws scrollbars in reverse video, so every background comparison
  uses the color actually drawn.
- **Scrollbars.** The vertical scrollbar is two columns wide, so scrollbar columns are dropped repeatedly, and only
  when they hold nothing but blanks and scrollbar glyphs. A horizontal scrollbar (Workflows at 100x30) is a row of
  blanks ending in a glyph, and ignoring trailing glyphs makes it an empty row.
- **Cursor in short tables.** "The row whose background differs from the others" has no answer with one or two rows,
  so bold breaks the tie.
- **Clipping.** Both sizes cut the Python footer off (`^p Comman`, `^v Paste lin`), and 100x30 cuts the last
  Workflows header. The structure records `footer_clipped` and `headers_clipped`, and both `[expect] footer` and the
  cross-app `footer`, `headers` and `rows` checks allow the cut (see Checks), so Rust doesn't have to clip at
  Textual's column.
- **Top-level tabs** go in the structure's `tabs` field, which the v1 example didn't have.

#### Rendering contract for the Rust UI

These rules double as the contract the Rust UI renders to, so that its screens are recognized the same way:

- Panes are light-border boxes (`┌┐└┘─│`) with the title on the top edge, drawn as `┌─ title ───┐`, and all border
  cells of a pane in one fg.
- The focused pane's border uses the theme's success color, and no unfocused border does.
- Colors are emitted as RGB (truecolor). An indexed color such as `Color::Green` dumps as `idx:2` and never equals a
  theme color like `#4ebf71`.
- Tabs are a label row with names separated by at least two blanks, over an underline row of `━╸╺`. ratatui's `Tabs`
  widget defaults to a `│` divider with single spaces, so it needs a custom label row.
- The underline spans the tab area's full width, in the inactive color everywhere except under the active tab. The
  active segment is found as "the cells not in the bar's most common fg", so a bar drawn only under the labels can
  name the wrong tab when the active label is wider than the others combined.
- A table's header row is bold on a background other than the pane's, and its style (background, bold) differs from
  the cursor row's: a header whose next row shares its style isn't a header, and the cursor is usually row 0.
- Columns start where the header's labels start and are separated by at least two blanks (ratatui's default
  `column_spacing` is 1, which merges columns; use 2). Every column has a non-empty header label, or its boundary
  disappears.
- The cursor row has its own background and is bold.
- The footer is the last row: each key in a style distinct from the labels, in Textual's notation (`^p`, `q`, `?`).

### Checks

`xtask/src/parity/check.rs` checks each capture in two ways and writes `<scenario>/<size>/<capture>.check.json`.
A capture with nothing to check (`check: "n/a"` in `summary.json`) gets no `check.json`, so the report shows it as
`not checked` rather than as having no mismatches.

```json
{"capture": "main", "size": [160, 48],
 "expect": [{"check": "titles", "box": null, "python": ["", "[9] Repositories", "…"], "rust": null,
             "expected": ["", "[1] Repositories", "…"], "message": "pane titles are […], expected […]"}],
 "cross": []}
```

A mismatch has `check`, `box` (the box's title, when it's about one box), `python`, `rust` and `message`. An `expect`
mismatch puts what the app showed under its own key and `null` under the other, and adds `expected`. After each size,
the console gets one line per mismatch.

- **`[expect.<capture>]`** is checked against each app that produced the capture:
  - `pane_titles` (check `titles`): every box title in pre-order, untitled boxes as `""`, exactly.
  - `focused` (check `focus`): the focused titles, exactly.
  - `footer`: the `"<key> <label>"` strings in order. When `footer_clipped` is set, the bindings shown must be a
    prefix of the expected list and the last one may be cut mid-label, so one full list works at every size.
  - `tabs`: the first box (in pre-order) whose title contains `box`. `names` and `active` must match when given.
  - `contains` / `not_contains`: searched in each row of the plain text, so they see everything on screen, modals
    included.

  A failure counts against that app. A Python failure always fails the run, because it means the scenario or the
  fixtures are wrong.
- **Cross-app**, Python against Rust, when both apps ran ok. Only the capture's `checks` run (default all):
  - `titles`: the ordered title lists are equal. One mismatch names the boxes found in only one app, or says that the
    order differs.
  - `tabs`: for each box title found in both apps, and for the top-level tabs, the names and the active tab are
    equal.
  - `headers`: per box, the list of its tables' `headers`. When either table has `headers_clipped`, the clipped-list
    rule below applies to that table's headers.
  - `rows`: per box, the number of tables (a table missing in one app is one mismatch, `python has N tables, rust
    has M`), then per table the row count, then each row cell by cell. Cells are trimmed, and a cell ending in `…`
    equals one that starts with its text before the `…`, because truncation width is layout, not content. When
    either table has `headers_clipped`, a row's last cells may also be one a prefix of the other.
  - `cursor`: per box and table, the `cursor` index. A differing number of tables is reported here when `rows`
    isn't checked.
  - `footer`: the `"<key> <label>"` lists. When either app has `footer_clipped`, the clipped-list rule applies.

  **Clipped lists** are compared up to the shorter list, and the last entry compared may be a prefix of the other
  app's: text cut at the screen's edge is layout, not content.
  - `focus`: the `focused` lists.

  Boxes are paired by title: the nth box with a title in one app with the nth in the other. A box found in only one
  app is reported once, under `titles`, and skipped by the other checks. Boxes whose title contains an `ignore.boxes`
  entry are skipped by every check but `titles`, and left out of the `focus` lists. Cross-app mismatches count against
  Rust.

### Reading the report

Every run ends by writing the human-readable report next to `summary.json` (`xtask/src/parity/report.rs`):

```
index.html                                       # every scenario × size, start here
summary.md                                       # the short table for the CI job summary (#70)
meta.json                                        # the run's commit, subject, time, OS, command and app sources
<scenario>/report.html                           # one page per scenario
<scenario>/<cols>x<rows>/<capture>.diff.txt      # unified diff of python/<capture>.txt against rust/<capture>.txt
```

Open `index.html` in a browser. It works straight from disk or from a downloaded CI artifact (`file://`): every page
is self-contained, with inline CSS, a few lines of inline JS and no external assets, fonts or CDNs.

**The published report.** The latest `rust-main` report is online at
<https://www.gizmo.codes/gh-lazy/parity/report/>, under a landing page at <https://www.gizmo.codes/gh-lazy/parity/>
that shows the commit it was built from (linked, with its subject) and when, the run's pass / fail / `rust-not-ready`
counts with the failed scenarios, the matrix's status by milestone and by area, its coverage (the
`cargo xtask parity-matrix` tables), and links to the matrix and PLAN §6.3. Only the latest report is kept. The
[Pages workflow](#ci) builds both; `cargo xtask parity-site --report <dir> --out <dir>` writes the landing page's
`index.html` locally from any report (the commit and time come from the report's `meta.json`, the counts from its
`summary.json`, so the page and the report can't disagree). It fails when the report has no `index.html`, and lists
the matrix's problems on the page rather than failing on them.

- **`index.html`** starts with the run's commit (`git rev-parse HEAD`), time, OS and command line, and where Python
  came from (`Python: recorded`, `Python: live`, `Python: live for a, b; recorded for the rest` or
  `Python: not run`), then one row per
  scenario × size: the result badge, each app's status, and the structural mismatch count (`not checked` when there's
  no `check.json`). Failed gated scenarios come first, then other failures, then `rust-not-ready`, then passes. Each
  row links to the scenario's page.
- **`<scenario>/report.html`** has a header with the description, `gate`, `parity_rows`, the result and a table of
  every app run per size: status, duration (`recorded` for a Python side read from its reference), flaky, the failed
  step, the exit code and fixture misses as `METHOD /path?query` keys. Then, per size and capture:
  - **Two panels**, Python on the left and Rust on the right, each the captured screen drawn from `grid.json` at one
    `ch` per cell (default colors `#d4d4d4` on `#1e1e1e`, inverse applied, indexed colors through the xterm palette).
    A panel wider than its half of the page scrolls sideways inside itself. The Python panel is captioned
    `Python (recorded)` when it shows a reference.
  - When an app has no capture, its panel says why (`exited with code 1 at step 2`, say) and shows `failure.grid.json`,
    the screen when the run failed, if there is one. An app that didn't run says `not run (--python-only)` or similar.
  - **Highlight differences** (when both apps captured) outlines every cell whose character differs between the two
    grids, and the count of differing cells sits next to it. The cells are marked when the report is written; the
    checkbox only flips a class, and it's hidden when JS is off.
  - **Structural mismatches**: the `check.json` table (check, box, Python value, Rust value, the `[expect]` value
    for an `expect` mismatch, message), tagged
    `expect`, `cross` or `drift`, or `not checked` when there's no `check.json`.
  - Collapsible sections: the text diff, both `structure.json` files pretty-printed, and per size the last 50 lines of
    each app's `fake-gh.log`. **Expand all** / **Collapse all** open or close every section at once.
- **`<capture>.diff.txt`** is written only when both apps captured. It's informational and never gates: Markdown,
  wrapping and spacing legitimately differ, and the structural check is what decides.
- **`summary.md`** is a heading `Parity: <n> pass, <n> fail, <n> rust-not-ready` (plus `python-only` / `rust-only`
  counts when there are any), the line `Python: …` as in `index.html`, a table
  `| Scenario | Gate | 160x48 | 100x30 | Mismatches |` (other sizes in the run get extra columns; a size a scenario
  didn't run is `skipped`; drift counts as mismatches), and a `<details>` list of the first 20 mismatches as
  `scenario/size/capture: check box: python=… rust=…` (`drift check box` for drift, where `python` is the live run and
  `rust` the reference), plus ` expected=…` for an `[expect]` mismatch, with values
  cut to 80 characters. A size where a run passed only on retry shows its result with ` (flaky)`. It stays under
  64 KiB.

A size's result in the index and `summary.md` is computed from that size's app statuses; a size with structural
mismatches takes the scenario's `fail` or `rust-not-ready` result.

### CI

The parity gate runs in the single CI job of `.github/workflows/rust.yaml`, `fmt, clippy, test (ubuntu-latest)`, on
every PR into `rust-main`, after fmt, clippy and the unit tests. It runs on Linux only, because macOS runners are too
expensive (PLAN §7). There's no `paths` filter: every PR runs the gate, and every PR runs the Rust app live. Python
runs only when the PR changes something it depends on, because a scenario's Python side is otherwise exactly its
[recorded reference](#recorded-references), which `cargo test` has already checked is fresh.

The checkout is two commits deep (`fetch-depth: 2`): a PR run checks out the PR's merge commit, whose first parent,
`HEAD^1`, is the base tip. A `Plan the parity run` step runs
`cargo xtask parity --all --live-python-since HEAD^1 --plan >> "$GITHUB_OUTPUT"`, and the rest of the job follows the
plan ([Running scenarios](#running-scenarios) has the rules):

| Plan | When | What runs |
| --- | --- | --- |
| `live-python=all` | The PR changes the Python app (`lazy_github/`, `pyproject.toml`, `uv.lock`), the capture harness, a pinned crate version, a Python-backed test (`xtask/tests/python_app.rs`, `xtask/tests/structural_check.rs`, `crates/fake-gh/tests/python_client.rs`), a file a [golden manifest](#manifest-format-v1) names, or the workflow. That's a merge of `main`, a harness change or a golden change. | Python 3.13 and `uv sync --frozen`, every scenario with Python live (checked for drift, so the re-recorded references are checked on the CI machine), and the `#[ignore]`d Python-backed tests, `cargo test -p fake-gh -p xtask -p lazy-github-tui -p lazy-github-core -- --ignored`, which include the [stale-golden checks](#the-staleness-test). |
| `live-python=a,b` | The PR re-recorded the references of `a` and `b`, and nothing above changed. | Python and `uv sync`; `a` and `b` run Python live, which checks that the developer's recording reproduces on Linux with Python 3.13. The rest read their references. No Python-backed tests. |
| `live-python=none` | Anything else: most feature PRs. | No `setup-uv`, no `uv sync`, no Python process. Rust runs against the references. |

The parity step is `cargo xtask parity --all --ci --no-sync --jobs 4 --live-python-since HEAD^1` (the runner's 4
vCPUs), which recomputes the same plan. The job summary starts with the plan line. Pushes to `rust-main` run only
fmt, clippy and the unit tests, because the squash-merged tree already passed the gate on its PR, and that run keeps
the Rust cache warm for PRs. The unit tests include the references freshness test, so a trunk that merged stale
references goes red at once.

A PR that edits a scenario, its fixtures or a harness input without re-recording fails in the Test step (stale
reference) and again in the parity step (exit `2`), with the `--record` command to run. Live runs on CI are the
check that the manifests hash every input: a missed input shows up as drift the next time the scenario runs live.
There's no scheduled live audit, since `schedule` runs only on the default branch, `main`; the parity reviewer's
milestone audits (`cargo xtask parity --all --live-python`) take that role.

**What gates.** The job fails when `cargo xtask parity` exits non-zero ([exit codes](#running-scenarios)) or an
ignored test fails:

- a scenario with `gate = true` has a structural mismatch, or an app run that failed at any size;
- the Python side failed in any scenario (timeout, crash, fixture miss, failed `[expect]`), since that means the
  fixtures or the scenario are broken;
- a scenario file is invalid or the harness failed.

A run that fails and then passes on its one retry doesn't fail the job; `summary.md` marks its size `(flaky)`.

**What doesn't gate.** The Rust side of a scenario with `gate = false` (`rust-not-ready`), the text diffs and the HTML
report. Because of `--ci`'s 3 s wait cap, a CI `rust-not-ready` can also mean a ready step that needs longer than 3 s.
Run the scenario locally without `--ci` before setting `gate = true`.

**Annotations.** With `--ci`, the last lines of the output are workflow commands that GitHub shows on the run and
on the scenario file in the PR:

- `::error file=parity/scenarios/<name>.toml::<scenario>/<size>/<capture>: <source> <check> <box>: <message>` for
  each mismatch that counts against the gate, where `<source>` is `<app> expect` for an `[expect]` mismatch on that
  app's screen or `cross` for a Python-vs-Rust one (the same wording as the console lines);
- `::error` for each app run that fails the gate (`<scenario>/<size>/<app>: timeout at step 0 (15.2s)`) and for a
  harness error: on the scenario file (and line) for an invalid scenario, and with no file for one that stops the
  whole run;
- one `::warning` per `rust-not-ready` scenario, listing the Rust failures and the number of mismatches, and one per
  run that passed only on retry.

**Flipping `gate`.** A feature PR that makes a scenario pass sets `gate = true` in that scenario file in the same
PR; its acceptance criteria say "scenario `<name>` with `gate = true`". The parity reviewer checks the flip, and
moves the rows whose ids are in `parity_rows` to `parity` in their `docs/rust-rewrite/parity/areas/<area>.md` files
when it merges. From then on a regression in that scenario fails every PR. The gate isn't read from the parity matrix,
so turning it on is atomic with the change that earns it.

**The report.** The job summary shows `summary.md` and a link to the `parity-report-<os>` artifact (kept 14 days).
Download it from the link (or the run page's **Artifacts** section), unzip it and open `index.html` in a browser; it
works from disk. With `gh`: `gh run download <run-id> -n parity-report-ubuntu-latest -D report && open
report/index.html`.

**Branch protection.** `fmt, clippy, test (ubuntu-latest)` is the one required check on `rust-main`. It covers the
parity gate too. The name predates the merged job and is kept so branch protection didn't have to change.

**Publishing (`.github/workflows/pages.yaml`).** A separate workflow, "Pages", builds and deploys the whole GitHub Pages
site: `main`'s README at <https://www.gizmo.codes/gh-lazy/>, built by `actions/jekyll-build-pages` as the legacy
branch build did, and the parity site under `/parity/` ([The published report](#reading-the-report)). It runs on a
push to `rust-main` that touches `crates/`, `xtask/`, `parity/`, `docs/rust-rewrite/parity/`, `Cargo.toml`,
`Cargo.lock`, `rust-toolchain.toml` or the workflow, and by hand (`workflow_dispatch`; with `deploy` off it only
builds the `github-pages` artifact). A newer run cancels one still running. It isn't a gate: it runs
`cargo xtask parity --all --no-sync --jobs 4` against the recorded references (no Python, no uv), ignores its exit
code so failing scenarios are published, then `cargo xtask parity-site`, and copies the report to `/parity/report/`.
Only a run with no report (no `index.html`) fails, which leaves the live site as it was. `main` is frozen during the
rewrite, so a change there is picked up by the next run, or a manual one.

### Recorded references

A recorded reference is the Python side of one scenario at one size, checked into the repo like an `insta`
snapshot, so a parity run can compare the Rust app against it without starting Python (#150). They are separate from
the data goldens in `parity/goldens/` (#157). `cargo xtask parity --record` writes them, `--check-references`
checks them, and a normal run reads them instead of starting Python ([Running scenarios](#running-scenarios));
`--live-python` runs Python and checks the references for drift.

```
parity/references/<scenario>/<cols>x<rows>/
  manifest.toml        # input hashes; written by --record, never by hand
  <capture>.screen     # one per capture step (format in #161)
  requests.jsonl       # the Python run's fake-gh log, normalized
```

Every scenario that runs Python has one directory per size in its `sizes`. Anything else under `parity/references/`
is an orphan: a directory for a scenario that doesn't exist or doesn't run Python, a size the scenario doesn't list,
a `.screen` for a capture it doesn't have, or any other file.

**The manifest.**

```toml
# Written by `cargo xtask parity --record`. Don't edit.
format = 1
size = "160x48"
captures = ["main"]

[inputs]
scenario = "3f1c..."
fixtures = "a09e..."
python_app = "77d2..."
harness = "c41b..."

[recorded_with]
python = "3.11.14"
os = "linux"
```

`captures` must equal the scenario's captures in step order. `recorded_with` is informational and never compared.
Each input is a git object id from `git hash-object`, run in the repo with a cleared environment:

| Input | What it hashes |
| --- | --- |
| `scenario` | The capture key: JSON of the resolved `config.json` (the config file plus `config_overrides`), the fixture set names in order, `gh_auth`, `git_remote`, `git_remote_protocol` (only when `"ssh"`), `git_branch`, `git_remote_branches` (only when set), `key_delay_ms`, `capture_stable_ms`, the size, every step and, only when the scenario sets one, the seed: its name and its files as sorted `[<path relative to the seed dir>, <blob>]` pairs. Waits are stored without `timeout_ms`, and captures with their `checks` and `ignore`. |
| `fixtures` | Every fixture in the scenario's sets whose request key (method, path, query, accept) equals a served entry in `requests.jsonl`, plus its `body_file`, as sorted `<set>/<relpath> <blob>` lines. For a `--paginate` request, the pages its `Link: rel="next"` headers lead to as well. |
| `python_app` | `lazy_github/`, `pyproject.toml` and `uv.lock`. |
| `harness` | `crates/fake-gh/src/`, `crates/fake-gh/Cargo.toml`, `xtask/src/parity/{env,runner,keys,grid}.rs`, `CAPTURE_VERSION` and the `vt100` and `portable-pty` versions in `Cargo.lock`. |

The file sets (`python_app`, `harness`) are the files `git ls-files -co --exclude-standard` lists (tracked, plus
untracked but not ignored: local edits count and `__pycache__` doesn't), each hashed from the working tree after
`.gitattributes` filters, so a CRLF checkout hashes like an LF one. The path lists and `CAPTURE_VERSION` are consts in
`xtask/src/parity/reference.rs`. Bump `CAPTURE_VERSION` when capture semantics change somewhere that isn't listed,
for example a new env var set in a file that isn't hashed.

**What invalidates a reference.**

| Change | References invalidated |
| --- | --- |
| Merging `main` into `rust-main` with changes to `lazy_github/`, `pyproject.toml` or `uv.lock` | all |
| A scenario's env, steps, `checks` or `ignore`, or the config file it uses | that scenario's |
| A file in a scenario's seed (`parity/seeds/<name>/`), added, changed or removed | that scenario's (`scenario changed`) |
| Adding or changing fixtures | those whose Python run requested a changed key |
| `crates/fake-gh/src/`, `env.rs`, `runner.rs`, `keys.rs` or `grid.rs`, a pinned crate, or `CAPTURE_VERSION` | all |
| `[expect]`, `gate`, `parity_rows`, `description`, timeouts, `apps`, `check.rs`, `structure.rs`, `report.rs` or `themes.toml` | none |

`[expect]` and the checks run against the stored screen on every run, which is why they need no re-record.

**Checking.** `cargo xtask parity --check-references [SCENARIO...]` recomputes the four hashes for each reference
and prints one line per problem, then a count. It needs only `git` and the fixtures (no build, no Python, no `uv`) and
takes well under a second.

```
stale reference: startup/160x48: fixtures, python_app changed; run cargo xtask parity --record startup
missing reference: startup/100x30; run cargo xtask parity --record startup
orphaned reference: parity/references/old-name/ (no such scenario); cargo xtask parity --record deletes it
invalid reference: startup/160x48/main.screen:1: expected `lazy-github-screen 1`, found "..."
```

A stale reference names every changed input, in the order `scenario, fixtures, python_app, harness`. A reference is
invalid when `manifest.toml` doesn't parse, has another `format` or `size`, or lists other captures than the
scenario, or when a `.screen` or `requests.jsonl` is missing or doesn't parse (a `.screen` is fully parsed, and the
error names its line). Orphaned directories of other scenarios are reported only when checking all of them. With
`--ci`, each problem is also an `::error` on the scenario file. `xtask/tests/references.rs` runs the same check in
`cargo test` and fails on any problem, missing references included, so a scenario change made without a re-record
fails the pre-push checks the way an outdated `insta` snapshot does.

#### Recording

`cargo xtask parity --record [SCENARIO...]` (no names means every scenario) builds `fake-gh`, runs
`uv sync --frozen --quiet` unless `--no-sync`, and then, for each scenario that runs Python, at each of its `sizes`:

- A fresh reference is skipped (`startup 160x48: fresh`) unless `--force`.
- Otherwise Python runs with **no retries and no wait caps**:
  1. If an old reference exists and this run agrees with it, the manifest is rewritten and each old `.screen` is kept
     when it's identical outside ignored boxes (every cell's text and style is equal except inside the interior of a
     box whose title contains one of the capture's `ignore.boxes`, in both screens). A re-record that changes nothing
     leaves only `recorded_with` in the diff: `startup 160x48: unchanged (1 run, 3.0s)`.
  2. Otherwise Python runs a second time, and run 1 is written if the two agree:
     `startup 160x48: recorded (2 runs, 6.2s)`.
  3. Otherwise nothing is written, and the first disagreement is printed with a unified diff of what it's about:
     `startup 160x48: not recorded: run 2 differs from run 1: main: rows [[1] Repositories]: ...`. A run that fails
     on its own isn't run again: `not recorded: run 1 timeout at step 3 (15.1s)` or
     `not recorded: run 1 [expect] main: contains: no row contains "..."`.

**Agree** means both runs passed (status `ok`, no fixture misses or errors), both pass the scenario's `[expect]`,
they produced the same captures, `check::compare` finds nothing between them for each capture's `checks` and
`ignore` (the cross-app check, between two Python runs; in its messages `python` is the first run and `rust` the
second), and they served the same request keys. `reference::agree` implements it, and the drift check of a live run
against its reference reuses it.
A failed run is never written and nothing is retried, so a flaky Python run shows up once, at record time, instead
of being stored.

A reference is written to `<size>.tmp/` and renamed into place, then checked for freshness. Afterwards `--record`
deletes orphans: with no names, directories of scenarios that don't exist or don't run Python; for each selected
scenario, sizes it doesn't list and `.screen` files of captures it doesn't have. Both runs' dumps stay in
`<out>/<scenario>/<size>/run-1/` and `run-2/` (the runner's usual files) for debugging. The exit code is `0` when
every selected reference is fresh at the end, `1` when some couldn't be recorded (the others are still written) and
`2` for a harness error or an invalid scenario.

**When a scenario won't record**, fix the scenario, not the tolerance: wait for a signal that follows the redraw
the capture races with, or add the box to `ignore.boxes` when the capture doesn't check its contents. Read the two
dumps first; a disagreement between two Python runs is a race the Rust side would be compared against too. Under
load (other parity runs at the same time), rerun with `--jobs 1` before changing anything.

Known races: after a repo is selected, Python sometimes draws the Details tab underline zero-width (`╸╺━━━` instead
of `╸━━━━╺`) and leaves it that way, so no wait settles it (#209). Captures taken after selecting a repo that don't
check the Details box list it in `ignore.boxes` (`table-lazy-load`, `status-bar-current-repo`), so a forced
re-record keeps the stored screen whichever form the run drew. The hidden terminal cursor's position varies too, but
no check reads it and a re-record doesn't compare it, so it never rewrites a screen.

Re-record a scenario, and commit `parity/references/`, whenever its env, steps, `checks` or `ignore` change. Flipping
`gate`, editing `parity_rows` or editing `[expect]` needs no re-record.

### Testing the harness

- `cargo test -p xtask` covers scenario parsing, key encoding, config layering, the query responder and the grid
  format, and drives a tiny test TUI (`xtask/tests/bin/echo_tui.rs`, built as an example) through the runner
  (`xtask/tests/runner.rs`). None of it needs Python. `xtask/src/parity/mouse.rs` unit-tests the SGR encoding and
  text targets (`nth`, no match, a match after a wide character). `echo_tui` turns on mouse capture (any-motion, or
  press/release only or off with `ECHO_TUI_MOUSE=press|off`) and echoes mouse events, and `tests/runner.rs` drives
  `click`, `wheel` and `move` through it, with a text target, an off-screen target, a run without motion reports and
  one with mouse reporting off.
- `xtask/tests/local_remote.rs` prepares runs with `git_remote_branches` and checks, with real git and fake-ssh
  (`xtask/tests/bin/fake_ssh.rs`, also an example), the bare repo's branches, `ls-remote` and `push` from `work/`
  with the app's environment, and `wait.remote_branch`. `crates/fake-gh/tests/ssh.rs` tests fake-ssh itself: both
  services, the refusals and the shared log lock.
- Structure extraction and the checks are unit-tested on synthetic grids in `xtask/tests/grids/*.txt` and on the real
  Python dumps in `xtask/tests/grids/python/` (golden `insta` snapshots in `xtask/src/parity/snapshots/`). A synthetic
  grid is the screen's characters, then optionally a `--- styles` layer with one letter per cell and a `--- legend`
  mapping letters to styles (`g fg=#4ebf71 bg=#121212 bold inverse`); `xtask/src/parity/test_grid.rs` documents the
  format. When the Python screen changes, refresh the dumps from a `--python-only` run and review the snapshot diff
  with `cargo insta review`.
- The report has `insta` snapshots (`summary.md`, the text diff, the HTML of a small grid pair) in
  `xtask/src/parity/snapshots/`, and `xtask/tests/report.rs` writes a whole report from synthetic dumps. Review
  snapshot changes with `cargo insta review`, or rerun with `INSTA_UPDATE=always` and read the diff.
- `xtask/tests/reference_reads.rs` records a reference with `echo_tui` standing in for Python in a temp repo, then
  checks that a normal run reads it with `--python /nonexistent` (`recorded: true`, checks run against it), that a
  stale one exits `2` before anything starts, and that `--live-python` finds no drift in an agreeing run and drift in
  a tampered reference. It also runs the `xtask` binary on a modified copy of `startup` for the stale lines. None of
  it needs Python.
- `xtask/tests/python_app.rs` runs `startup` against the Python app and checks that a fixture miss fails the run. It
  needs uv, so it's `#[ignore]`d (CI runs it after the parity scenarios): run it with
  `cargo build -p fake-gh && cargo test -p xtask --test python_app -- --ignored`. `xtask/tests/structural_check.rs`
  (also ignored) rebinds `main.repos.focus` to `9` in a copy of `startup` and checks that `[expect]` fails with a
  `titles` mismatch naming `[9] Repositories` and exit code 1.

## Python goldens

Some parity rows are about data, not screens: binding ids and their defaults, config defaults, labels, key spellings,
parsing and file formats. For those, a script imports the Python app and dumps what it has (a **golden**, committed
next to the Rust test), and a plain Rust test compares the Rust side with it in `cargo test`. A manifest in
`parity/goldens/<name>.toml`, parallel to `parity/scenarios/`, links the golden to the parity matrix.

### Golden or scenario

Use a golden for data: ids, defaults, labels, key spellings, parsing and file formats. A golden checks every entry at
once, and doesn't need a screen. Use a scenario for anything that needs a screen or a key press to have an effect:
what's drawn, where focus goes, what a key does. A row can be named by both, when the golden checks its data and a
scenario its effect.

### Manifest format (v1)

```toml
version = 1
description = "Binding registry: every id's default keys, action, label, tooltip, show and key_display"
generator = "crates/lazy-github-tui/tests/fixtures/generate_bindings.py"
golden = ["crates/lazy-github-tui/tests/fixtures/bindings.json"]
test = "crates/lazy-github-tui/tests/registry.rs"
fresh_test = "bindings_goldens_are_fresh"
parity_rows = ["keybindings.all-ids"]
```

| Key | Meaning |
| --- | --- |
| `version` | Always `1`. |
| `description` | What the golden holds, in one line. |
| `generator` | The Python script that writes the golden. It takes `--out DIR` (default: its own directory). |
| `golden` | The files the generator writes, all of them. |
| `test` | The Rust test file that compares the Rust side with the golden in plain `cargo test`. |
| `fresh_test` | The `#[ignore]`d `#[test]` fn in `test` that regenerates the golden and compares it ([below](#the-staleness-test)). |
| `parity_rows` | The row ids the golden covers. |

Paths are relative to the repository root. Unknown keys are an error, so a typo like `parity_row` can't silently cover
nothing. `parity_rows` is an explicit list; globs aren't allowed. A row goes in only when this golden checks every
column and note of the row, together with any scenario that also names it. A feature PR that completes a golden's
check for a row adds the id in that PR.

`cargo xtask parity-matrix` (and `cargo test --workspace`, through `xtask/tests/parity_matrix.rs`) reports a manifest
that doesn't parse, has an unknown key or a `version` other than 1, a `parity_rows` entry that isn't a row id, a
`generator`, `golden` or `test` file that doesn't exist, a `test` file with no `#[test]` that isn't `#[ignore]`d, and
a `fresh_test` that isn't an `#[ignore]`d `#[test]` fn in `test`. The `#[test]` and `#[ignore]` detection is a line
scan: a line `#[test]` starts an attribute block that ends at the next `fn ` or `async fn ` line, and the fn is ignored
if a line in the block starts with `#[ignore`. Write the golden tests as plain `#[test]` fns, one attribute per line.

The coverage table after the status table counts each area's rows named by a scenario only, a golden only, both, or
neither; `cargo xtask parity-matrix --uncovered` lists the neither rows (except intentional diffs).

### Regenerating a golden

The Python app is frozen on `rust-main`, so a golden changes only when its generator does (for example, when a
feature adds a field to the dump). Run the generator from the repository root with the `HOME` its docstring names, so
the user's own config doesn't leak in, e.g.:

```sh
HOME=$(mktemp -d) uv run python crates/lazy-github-tui/tests/fixtures/generate_bindings.py
HOME=/home/lazy uv run python crates/lazy-github-core/tests/fixtures/config/generate.py
```

Commit the generator change and the regenerated golden together. Run with no arguments, a generator must leave
`git status` clean.

### The staleness test

Each manifest's `fresh_test`, `#[ignore = "needs uv and the Python app"]` in its `test` file, runs the generator with
`--out <tempdir>` from the repository root, with the `HOME` its docstring names, and compares every `golden` file byte
for byte. On a mismatch it fails with the first 40 lines of a unified diff and the regenerate command. (It asks
`uv run --frozen python` for the project's interpreter under the real `HOME`, so uv's cache is the usual one, then runs
the generator with that interpreter and the test's `HOME`.) The config generator also checks that the hand-written
config fixtures load in Python; its non-zero exit fails the test.

CI runs these in the "Python-backed integration tests" step,
`cargo test --locked -p fake-gh -p xtask -p lazy-github-tui -p lazy-github-core -- --ignored`, on every PR whose
[parity plan](#ci) is `live-python=all`. A change to the Python app or to any `generator`, `golden` or `test` file a
manifest names makes it `all` (the plan reads the manifests). So every `#[ignore]`d
test in `lazy-github-tui` and `lazy-github-core` must be safe to run there. Locally:

```sh
cargo test -p lazy-github-tui -p lazy-github-core -- --ignored
```

The parity reviewer moves a row in a golden's `parity_rows` to `parity` when the manifest's `test` passes in
`cargo test` and its `fresh_test` passed in the PR's CI run, with the exceptions in
[Editing the matrix](../docs/rust-rewrite/parity/README.md#editing-the-matrix).
