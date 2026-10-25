# Parity fixtures and fake-gh

Both LazyGithub implementations reach GitHub only through the `gh` CLI. For parity runs and tests, `crates/fake-gh` stands
in for `gh` and answers from the fixture files in `parity/fixtures/`, so the Python and Rust apps see byte-identical
data with no network. See `docs/rust-rewrite/PLAN.md` §6.2.

- [fake-gh](#fake-gh): installing it, environment variables, supported commands, output, the request log.
- [Fixture format (v1)](#fixture-format-v1): layout, file schema, key normalization, matching.
- [`fake-gh lint`](#fake-gh-lint).

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

## Overlays

- `repos-two-pages/`: base's three repos split over two `GET /user/repos` pages (two, then one) linked by
  `Link: rel="next"`. Listed before `base`, it exercises pagination; `crates/fake-gh/tests/rust_client.rs` uses it.
