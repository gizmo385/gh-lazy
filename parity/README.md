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
  the scenario format, screen dumps and results.

## fake-gh

`fake-gh` is a drop-in `gh` binary. Build it with `cargo build -p fake-gh`, install `target/debug/fake-gh` as
`<tmp>/bin/gh` (`gh.exe` on Windows), and put that directory first on `PATH`. From Rust, `fake_gh::install(binary,
dir)` does the install: it hard-links when it can, and otherwise copies and waits out the `ETXTBSY` race that a
freshly copied executable has in a multi-threaded process. The binary is deliberately not named
`gh`, so `target/debug` on `PATH` never shadows the real gh.

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

Some recorded data depends on who records it: a repo object's `permissions` reflects the recorder's token, and admins
also get `security_and_analysis`. The recorder pins every `permissions` object to read-only (`pull` only) and drops
`security_and_analysis`, so a recording looks the same whoever makes it. `check-fixtures` doesn't flag either, since
hand-written fixtures such as `/user/repos` model a user who owns their repos.

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

Runs the same secret, email, avatar, `private` and header rules over every fixture under `parity/fixtures/`
(hand-written ones included), without network, and exits 1 on any finding: anything the scrubber would have
rewritten or refused. `body_file`s that aren't UTF-8 are treated as binary and skipped. `cargo test` runs it
(`xtask/src/fixtures/check.rs`), so CI enforces it.

## The parity harness (`cargo xtask parity`)

`cargo xtask parity` runs scenarios from `parity/scenarios/` against the Python app and the Rust app, each in its own
pseudo-terminal with an isolated `HOME` and fake-gh on `PATH`, and dumps the screen at each capture point. See
`docs/rust-rewrite/PLAN.md` §6.3. The code is in `xtask/src/parity/`.

### Running scenarios

```
cargo xtask parity [SCENARIO...] [--all] [--python-only | --rust-only] [--size COLSxROWS]...
                   [--out DIR] [--keep-home] [--no-build] [--no-sync] [--python PATH]
                   [--retries N] [--jobs N] [--ci] [--validate]
```

```sh
cargo xtask parity startup --python-only   # the Python side of one scenario
cargo xtask parity                         # every scenario, both apps
cargo xtask parity --validate              # check every scenario file, run nothing
```

- `SCENARIO` is a file stem from `parity/scenarios/` (`startup`) or a path to a `.toml` file. With none given,
  `--all` is implied.
- Before running, xtask builds `lazy-github` and `fake-gh` (`cargo build -p lazy-github -p fake-gh`) unless
  `--no-build`. It also runs `uv sync --frozen --quiet` under your real environment unless `--no-sync` (or no Python
  run is needed). uv is never run under the isolated `HOME`, where it would start from an empty cache.
- Each scenario runs once per size and per app, in a fresh isolated environment. Sizes and apps within a scenario run
  one after another.
- The report goes to `parity/reports/latest/` (wiped first, and git-ignored) or to `--out DIR`. With `--out`, only the
  directories of the scenarios being run are replaced.

| Flag | Meaning |
| --- | --- |
| `--all` | Run every scenario in `parity/scenarios/`. |
| `--python-only`, `--rust-only` | Run one app only: for capturing references and for debugging. |
| `--size COLSxROWS` | Run at this size instead of the scenarios' `sizes`. Repeatable. |
| `--out DIR` | Write the report here instead of `parity/reports/latest/`. |
| `--keep-home` | Copy each run's `home/` and `work/` into its report directory. |
| `--no-build` | Skip `cargo build -p lazy-github -p fake-gh`. The binaries must already be in `target/<profile>/`. |
| `--no-sync` | Skip `uv sync --frozen`. |
| `--python PATH` | Run `PATH -m lazy_github` instead of the repo's `.venv/bin/python` (`.venv\Scripts\python.exe` on Windows). |
| `--retries N` | Re-run a failed app run up to N times. A run that passes on retry is marked `flaky: true`. Default 0. |
| `--jobs N` | Run up to N scenarios in parallel. Default 1. |
| `--ci` | Running in CI. Accepted now; CI-specific output arrives with #70. |
| `--validate` | Parse and validate the scenario files (all, or the ones named) and exit. |

Exit codes:

- `0`: every gated scenario passed and the Python side is clean everywhere.
- `1`: a gated scenario failed, or the Python side failed anywhere (timeout, crash, fixture miss).
- `2`: a scenario file is invalid (printed as `file:line: message`), or the harness itself failed.

Rust-side failures in scenarios with `gate = false` are reported as `rust-not-ready` and don't change the exit code,
so scenarios can land before the Rust UI exists. Until M1 the Rust binary just exits, so every scenario's Rust side
is `rust-not-ready`.

### The isolated environment

Every scenario × size × app gets a temp dir `<run>` holding `home/`, `bin/` and `work/`. It's removed after the run;
`--keep-home` copies `home/` and `work/` into the report first.

- `home/.config/lazy-github/config.json` is built from three layers, in order:
  - the scenario's `config` file, `parity/configs/<name>.json` (default `default`);
  - deep-merged with the scenario's `config_overrides` (objects merge key by key; anything else replaces);
  - then always forced: `core.first_start = false` and `api.client_type = "GITHUB_CLI"`, which keep the Python app
    off its first-start screen and on the `gh` backend.

  The Python app keeps everything (config, log, table cache, PR drafts) under `$HOME/.config/lazy-github`, so a fresh
  `HOME` also means no cached tables are shown before the first fetch.
- `bin/gh` (`gh.exe` on Windows) is fake-gh, installed with `fake_gh::install`.
- `work/` is the app's working directory. When the scenario sets `git_remote = "owner/name"`, it's made a git repo:
  - `git init -q -b <git_branch, default main>`
  - `git remote add origin https://github.com/owner/name.git`
  - `git commit -q --allow-empty -m parity` as `parity-bot <parity-bot@example.com>` dated `2026-01-01T00:00:00Z`,
    so the commit SHA is the same on every run.
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
parity_rows = ["Main layout (status bar, selections pane 40%, details, command log, footer)"]
                                 # PARITY.md "Item" texts; informational, shown in the report
sizes = [[160, 48], [100, 30]]   # [cols, rows]; default [[160, 48]]
timeout_ms = 30000               # whole run per app per size; default 30000
key_delay_ms = 30                # pause after each key; default 30 (after "escape" always >= 100)
capture_stable_ms = 300          # implicit stability wait before each capture; default 300

[env]
fixtures = ["base"]              # sets under parity/fixtures/, earlier overrides later; default ["base"]
config = "default"               # parity/configs/<name>.json; default "default"
gh_auth = "logged-in"            # or "logged-out"; default "logged-in"
git_remote = "gizmo385/gh-lazy"  # optional; omit = CWD is not a git repo
git_branch = "main"              # optional, default "main"

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
  a keymap design question (PARITY.md, "Known terminal caveats").
- `[expect.<capture>]` keys must name a capture in the scenario. All of them are optional. `pane_titles` and `focused`
  are exact ordered lists. `footer` is an exact ordered list of `"<key> <label>"`.
- Until #93 lands, `checks`, `ignore` and `[expect]` are parsed and validated but not evaluated.

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
- Steps run in order. Waits poll the screen every 20 ms. Each key is followed by `key_delay_ms` (at least 100 ms after
  `escape`, so Textual's escape-sequence timeout doesn't fuse it with the next key).
- The whole run is bounded by `timeout_ms`. On a step timeout, a crash, or an exit before the last step, the runner
  writes `failure.txt` and `failure.grid.json` (the screen at that moment) and records the status.
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
<scenario>/<cols>x<rows>/<app>/home/, work/      # only with --keep-home
```

The structural check (#93) adds `<capture>.structure.json` and `<capture>.check.json`, and the report (#94) adds
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
  before the last step) or `spawn-error` (the PTY or the app couldn't be started; `message` says why).
- `step` is the index of the failing step, and `exit_code` the app's exit code when it exited.
- `misses` are the fake-gh log lines with `"miss": true`, ready for `cargo xtask record-fixtures --from-log` (#68), and
  `errors` the lines with an `error`. Either makes `fixture_problems: true`, and the run counts as failed even though
  its status is `ok`.

`summary.json` collects everything:

```json
{"version": 1, "scenarios": [{"name": "startup", "description": "…", "gate": false, "parity_rows": ["…"],
  "result": "rust-not-ready",
  "sizes": [{"size": "160x48", "apps": {"python": <status>, "rust": <status>}, "captures": [{"name": "main"}]}]}]}
```

An app that didn't run is left out of `apps`. A scenario's `result` is:

- `pass`: both apps ran and passed.
- `fail`: the Python side failed, or a gated scenario's Rust side did. `error` is set when the harness itself failed.
- `rust-not-ready`: not gated, and the Rust side failed.
- `python-only` / `rust-only`: only one app ran (a flag or the scenario's `apps`), and it passed.

Until #93 lands, results come from app statuses only.

### Testing the harness

- `cargo test -p xtask` covers scenario parsing, key encoding, config layering, the query responder and the grid
  format, and drives a tiny test TUI (`xtask/tests/bin/echo_tui.rs`, built as an example) through the runner
  (`xtask/tests/runner.rs`). None of it needs Python.
- `xtask/tests/python_app.rs` runs `startup` against the Python app and checks that a fixture miss fails the run. It
  needs uv, so it's `#[ignore]`d: run it with `cargo build -p fake-gh && cargo test -p xtask --test python_app --
  --ignored`.
