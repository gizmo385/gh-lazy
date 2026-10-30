//! The scrub rules: what a recorded response may keep, what gets rewritten, and what aborts a recording.
//!
//! [`scrub`] applies them to a fresh `gh api -i` response. [`Rules::check_fixture`] runs the same rules read-only over
//! a committed fixture, for `cargo xtask check-fixtures`: anything the scrubber would rewrite or refuse is a finding.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use fake_gh::fixture::{Fixture, canonical_header_name};
use regex::{Captures, Regex};
use serde_json::Value;

use super::recipe::RecipeEntry;

/// The headers a fixture keeps. Everything else (`Date`, `Etag`, rate limits, request ids, OAuth details) is
/// volatile or describes the recorder's token.
pub const ALLOWED_HEADERS: [&str; 3] = ["Content-Type", "Link", "Location"];

/// The rate-limit headers a 403 or 429 keeps as well: on those statuses they say whether GitHub is rate limiting and
/// until when, which is what the response means (#324).
pub const RATE_LIMIT_HEADERS: [&str; 5] = [
    "Retry-After",
    "X-Ratelimit-Limit",
    "X-Ratelimit-Remaining",
    "X-Ratelimit-Reset",
    "X-Ratelimit-Resource",
];

/// Whether a response with `status` keeps the header `name` (canonical).
fn header_allowed(name: &str, status: u16) -> bool {
    ALLOWED_HEADERS.contains(&name) || (matches!(status, 403 | 429) && RATE_LIMIT_HEADERS.contains(&name))
}

/// What emails become.
pub const PLACEHOLDER_EMAIL: &str = "user@example.com";

/// What `avatar_url` becomes.
pub const PLACEHOLDER_AVATAR: &str = "https://avatars.githubusercontent.com/u/0?v=4";

/// Text emails at these domains are already safe and kept.
const SAFE_EMAIL_DOMAINS: [&str; 2] = ["example.com", "users.noreply.github.com"];

/// JSON keys whose string values are emails.
const EMAIL_KEYS: [&str; 3] = ["email", "author_email", "committer_email"];

/// A response as `gh api -i` printed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawResponse {
    pub status: u16,
    /// Header names as printed, in order.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// `gh api -i` output that has no `HTTP/<version> <code>` status line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("gh output has no HTTP status line (starts with {start:?})")]
pub struct NoStatusLine {
    pub start: String,
}

impl RawResponse {
    /// Parses `gh api -i` output: a status line `HTTP/<any version> <code> ...`, headers up to the first empty line,
    /// then the body as bytes. gh ends the status line with LF and headers with CRLF; either is accepted.
    pub fn parse(output: &[u8]) -> Result<RawResponse, NoStatusLine> {
        let no_status = || NoStatusLine {
            start: String::from_utf8_lossy(&output[..output.len().min(60)]).into_owned(),
        };
        let mut rest = output;
        let status_line = next_line(&mut rest).ok_or_else(no_status)?;
        let status = std::str::from_utf8(status_line)
            .ok()
            .and_then(|line| {
                let mut parts = line.split_ascii_whitespace();
                let version = parts.next()?;
                version.starts_with("HTTP/").then_some(())?;
                parts.next()?.parse::<u16>().ok()
            })
            .ok_or_else(no_status)?;

        let mut headers = Vec::new();
        while let Some(line) = next_line(&mut rest) {
            if line.is_empty() {
                break;
            }
            let line = String::from_utf8_lossy(line);
            if let Some((name, value)) = line.split_once(':') {
                headers.push((name.trim().to_owned(), value.trim().to_owned()));
            }
        }
        Ok(RawResponse {
            status,
            headers,
            body: rest.to_vec(),
        })
    }
}

/// Splits off one line, without its `\n` or `\r\n`. `None` when there's no complete line left.
fn next_line<'a>(rest: &mut &'a [u8]) -> Option<&'a [u8]> {
    let end = rest.iter().position(|&b| b == b'\n')?;
    let line = &rest[..end];
    *rest = &rest[end + 1..];
    Some(line.strip_suffix(b"\r").unwrap_or(line))
}

/// A response that passed scrubbing.
#[derive(Debug, Clone, PartialEq)]
pub struct ScrubbedResponse {
    pub status: u16,
    /// Allow-listed headers, Go-canonical names.
    pub headers: BTreeMap<String, String>,
    pub body: ScrubbedBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScrubbedBody {
    Empty,
    Json(Value),
    Text(String),
    /// Only for recipe entries marked `binary = true`. Never scanned.
    Binary(Vec<u8>),
}

/// Where in a response a finding is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Header(String),
    /// An RFC 6901 pointer into the JSON body; `""` is the whole body.
    Json(String),
    /// A byte offset into a text body.
    Byte(usize),
    /// The body as a whole.
    Body,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::Header(name) => write!(f, "header {name}"),
            Location::Json(pointer) => write!(f, "JSON pointer {pointer:?}"),
            Location::Byte(offset) => write!(f, "byte offset {offset}"),
            Location::Body => f.write_str("body"),
        }
    }
}

/// What a rule found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Issue {
    /// A string matching a secret pattern. Aborts: secrets are never rewritten, the recipe is wrong.
    Secret(&'static str),
    /// An object with `"private": true`. Aborts.
    Private,
    /// A body that isn't UTF-8 from an entry not marked `binary = true`. Aborts.
    NotUtf8,
    /// A header outside [`ALLOWED_HEADERS`]. The scrubber drops it.
    Header,
    /// An email that the scrubber replaces with [`PLACEHOLDER_EMAIL`].
    Email,
    /// An `avatar_url` or `gravatar_id` that the scrubber replaces with its placeholder.
    Avatar,
    /// Data that describes the recorder's own access to a repo: a `permissions` object other than read-only, or a
    /// key that GitHub only shows to push or admin viewers ([`VIEWER_ONLY_KEYS`]). When recording, the scrubber pins
    /// the first and drops the second. `check-fixtures` doesn't flag either, so hand-written fixtures can model an owner.
    Viewer,
    /// A `Link` URL at a numeric-id path (`/repositories/<id>`, `/user/<id>`, `/organizations/<id>`). The scrubber
    /// maps it to the request's path when the rest of the path matches, and otherwise leaves it for a person to fix.
    IdLink,
}

impl Issue {
    /// Whether this finding stops a recording, rather than being rewritten.
    pub fn aborts(&self) -> bool {
        matches!(self, Issue::Secret(_) | Issue::Private | Issue::NotUtf8)
    }
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Issue::Secret(name) => write!(f, "looks like a secret ({name})"),
            Issue::Private => f.write_str(r#"object has "private": true"#),
            Issue::NotUtf8 => {
                f.write_str("body is not UTF-8; mark the recipe entry `binary = true` if that's expected")
            }
            Issue::Header => write!(f, "header not in the allow-list ({})", ALLOWED_HEADERS.join(", ")),
            Issue::Email => write!(f, "email should be {PLACEHOLDER_EMAIL}"),
            Issue::Avatar => write!(f, "avatar should be the placeholder ({PLACEHOLDER_AVATAR} or \"\")"),
            Issue::Viewer => f.write_str(
                "depends on who recorded: `permissions` should be read-only and push/admin-only repo keys absent",
            ),
            Issue::IdLink => {
                f.write_str("points at a numeric-id path (/repositories/<id>/...); should be the request's path")
            }
        }
    }
}

/// One rule hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub location: Location,
    pub issue: Issue,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.issue)
    }
}

/// Why [`scrub`] refused a response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .findings.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
pub struct ScrubAbort {
    /// The aborting findings, in body order. Never empty.
    pub findings: Vec<Finding>,
}

/// Scrubs one recorded response for `entry`. Pure: recording is I/O plus a call to this.
pub fn scrub(resp: RawResponse, entry: &RecipeEntry) -> Result<ScrubbedResponse, ScrubAbort> {
    let rules = Rules::get();
    let mut findings = Vec::new();
    let mut headers = rules.filter_headers(resp.headers, resp.status, &mut findings);
    if let Some(link) = headers.get_mut("Link") {
        let (rewritten, flagged) = rewrite_id_links(link, &entry.path);
        if flagged {
            findings.push(Finding {
                location: Location::Header("Link".into()),
                issue: Issue::IdLink,
            });
        }
        *link = rewritten;
    }
    let content_type = headers.get("Content-Type").map_or("", String::as_str);

    let body = if resp.body.is_empty() {
        ScrubbedBody::Empty
    } else if entry.binary {
        ScrubbedBody::Binary(resp.body)
    } else {
        match String::from_utf8(resp.body) {
            Err(_) => {
                findings.push(Finding {
                    location: Location::Body,
                    issue: Issue::NotUtf8,
                });
                ScrubbedBody::Empty
            }
            Ok(text) => match is_json_content_type(content_type)
                .then(|| serde_json::from_str::<Value>(&text).ok())
                .flatten()
            {
                Some(mut value) => {
                    rules.scrub_json(&mut value, &mut String::new(), &mut findings, true);
                    ScrubbedBody::Json(value)
                }
                None => ScrubbedBody::Text(rules.scrub_text(&text, &mut findings)),
            },
        }
    };

    let aborts: Vec<Finding> = findings.into_iter().filter(|f| f.issue.aborts()).collect();
    if aborts.is_empty() {
        Ok(ScrubbedResponse {
            status: resp.status,
            headers,
            body,
        })
    } else {
        Err(ScrubAbort { findings: aborts })
    }
}

fn is_json_content_type(content_type: &str) -> bool {
    let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    mime == "application/json" || mime.ends_with("+json")
}

/// The compiled rule set, shared by the scrubber and the scrub check.
#[derive(Debug)]
pub struct Rules {
    secrets: Vec<(&'static str, Regex)>,
    email: Regex,
    /// One `<url>` in a `Link` value.
    link_url: Regex,
}

impl Rules {
    /// The rule set, compiled once.
    pub fn get() -> &'static Rules {
        static RULES: OnceLock<Rules> = OnceLock::new();
        RULES.get_or_init(|| {
            // The patterns are constants, so a compile failure is a bug caught by every test.
            let re = |p: &str| Regex::new(p).expect("scrub pattern compiles");
            Rules {
                secrets: vec![
                    ("GitHub token", re(r"gh[pousr]_[A-Za-z0-9]{36,}")),
                    ("GitHub fine-grained token", re(r"github_pat_[A-Za-z0-9_]{22,}")),
                    ("private key", re(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
                    ("AWS access key", re(r"AKIA[0-9A-Z]{16}")),
                    ("Slack token", re(r"xox[baprs]-[A-Za-z0-9-]{10,}")),
                ],
                email: re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
                link_url: re(r"<([^>]*)>"),
            }
        })
    }

    /// The first secret pattern in `s`, with the byte offset of the match.
    fn find_secret(&self, s: &str) -> Option<(&'static str, usize)> {
        self.secrets
            .iter()
            .filter_map(|(name, re)| re.find(s).map(|m| (*name, m.start())))
            .min_by_key(|&(_, offset)| offset)
    }

    /// Keeps allow-listed headers of a `status` response under their canonical names (repeats joined with `, `).
    /// Dropped headers are findings; a secret in a kept header aborts.
    fn filter_headers(
        &self,
        headers: Vec<(String, String)>,
        status: u16,
        findings: &mut Vec<Finding>,
    ) -> BTreeMap<String, String> {
        let mut kept: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in headers {
            let name = canonical_header_name(&name);
            if !header_allowed(&name, status) {
                findings.push(Finding {
                    location: Location::Header(name),
                    issue: Issue::Header,
                });
                continue;
            }
            if let Some((secret, _)) = self.find_secret(&value) {
                findings.push(Finding {
                    location: Location::Header(name.clone()),
                    issue: Issue::Secret(secret),
                });
            }
            kept.entry(name)
                .and_modify(|v| {
                    v.push_str(", ");
                    v.push_str(&value);
                })
                .or_insert(value);
        }
        kept
    }

    /// Rewrites emails and avatars in place and records every hit. `pointer` is the location of `value`. With
    /// `pin_viewer`, recorder-specific repo access is also pinned or dropped (see [`Issue::Viewer`]); that's for
    /// recordings only, since hand-written fixtures may model a user who owns their repos.
    fn scrub_json(&self, value: &mut Value, pointer: &mut String, findings: &mut Vec<Finding>, pin_viewer: bool) {
        match value {
            Value::String(s) => {
                if let Some((secret, _)) = self.find_secret(s) {
                    findings.push(Finding {
                        location: Location::Json(pointer.clone()),
                        issue: Issue::Secret(secret),
                    });
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter_mut().enumerate() {
                    let len = pointer.len();
                    pointer.push('/');
                    pointer.push_str(&i.to_string());
                    self.scrub_json(item, pointer, findings, pin_viewer);
                    pointer.truncate(len);
                }
            }
            Value::Object(map) => {
                if map.get("private") == Some(&Value::Bool(true)) {
                    findings.push(Finding {
                        location: Location::Json(pointer.clone()),
                        issue: Issue::Private,
                    });
                }
                if pin_viewer {
                    for key in VIEWER_ONLY_KEYS.iter().filter(|key| map.contains_key(**key)) {
                        findings.push(Finding {
                            location: Location::Json(format!("{pointer}/{key}")),
                            issue: Issue::Viewer,
                        });
                    }
                    // `retain` keeps the order of the other keys; `remove` would swap the last key into its place.
                    map.retain(|key, _| !VIEWER_ONLY_KEYS.contains(&key.as_str()));
                }
                for (key, item) in map.iter_mut() {
                    let len = pointer.len();
                    pointer.push('/');
                    pointer.push_str(&key.replace('~', "~0").replace('/', "~1"));
                    if let Some((secret, _)) = self.find_secret(key) {
                        findings.push(Finding {
                            location: Location::Json(pointer.clone()),
                            issue: Issue::Secret(secret),
                        });
                    }
                    // Secrets are looked for before any rewrite, so a token in an email field still aborts.
                    self.scrub_json(item, pointer, findings, pin_viewer);
                    if let Some(issue) = replace_known_field(key, item, pin_viewer) {
                        findings.push(Finding {
                            location: Location::Json(pointer.clone()),
                            issue,
                        });
                    }
                    pointer.truncate(len);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// Replaces emails outside [`SAFE_EMAIL_DOMAINS`] and records every hit. The rest is kept byte for byte.
    fn scrub_text(&self, text: &str, findings: &mut Vec<Finding>) -> String {
        for (name, re) in &self.secrets {
            for m in re.find_iter(text) {
                findings.push(Finding {
                    location: Location::Byte(m.start()),
                    issue: Issue::Secret(name),
                });
            }
        }
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for m in self.email.find_iter(text) {
            if !is_personal_email(m.as_str()) {
                continue;
            }
            findings.push(Finding {
                location: Location::Byte(m.start()),
                issue: Issue::Email,
            });
            out.push_str(&text[last..m.start()]);
            out.push_str(PLACEHOLDER_EMAIL);
            last = m.end();
        }
        out.push_str(&text[last..]);
        findings.sort_by_key(|f| match f.location {
            Location::Byte(offset) => offset,
            _ => 0,
        });
        out
    }

    /// Runs every rule over a committed fixture without changing it. Each finding is something the scrubber would
    /// have rewritten or refused. `body_file` is the contents of the fixture's `body_file`, if it has one; a body
    /// file that isn't UTF-8 is treated as binary and not scanned.
    pub fn check_fixture(&self, fixture: &Fixture, body_file: Option<&[u8]>) -> Vec<Finding> {
        let mut findings = Vec::new();
        let headers = fixture
            .response
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let kept = self.filter_headers(headers, fixture.response.status, &mut findings);
        if let Some(link) = kept.get("Link")
            && rewrite_id_links(link, &fixture.request.path).1
        {
            findings.push(Finding {
                location: Location::Header("Link".into()),
                issue: Issue::IdLink,
            });
        }
        if let Some(body) = &fixture.response.body {
            self.scrub_json(&mut body.clone(), &mut String::new(), &mut findings, false);
        }
        if let Some(text) = &fixture.response.body_text {
            self.scrub_text(text, &mut findings);
        }
        if let Some(text) = body_file.and_then(|bytes| std::str::from_utf8(bytes).ok()) {
            self.scrub_text(text, &mut findings);
        }
        findings
    }
}

/// A numeric-id path form that GitHub writes in `Link` URLs, and the request path it stands for.
struct IdAlias {
    /// `/repositories/`, followed by the id.
    id_prefix: &'static str,
    /// `/repos/`, followed by `names` slash-separated name segments.
    named_prefix: &'static str,
    names: usize,
}

const ID_ALIASES: [IdAlias; 3] = [
    IdAlias {
        id_prefix: "/repositories/",
        named_prefix: "/repos/",
        names: 2,
    },
    IdAlias {
        id_prefix: "/user/",
        named_prefix: "/users/",
        names: 1,
    },
    IdAlias {
        id_prefix: "/organizations/",
        named_prefix: "/orgs/",
        names: 1,
    },
];

impl IdAlias {
    /// The alias `path` uses, and the rest of the path after the id (`""` or starting with `/`).
    fn split(path: &str) -> Option<(&'static IdAlias, &str)> {
        ID_ALIASES.iter().find_map(|alias| {
            let after = path.strip_prefix(alias.id_prefix)?;
            let (id, rest) = after.split_at(after.find('/').unwrap_or(after.len()));
            (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some((alias, rest))
        })
    }

    /// The rest of `request_path` after the part the id stands for (`/repos/<owner>/<repo>`), if it has that shape.
    fn rest_of<'a>(&self, request_path: &'a str) -> Option<&'a str> {
        let tail = request_path.strip_prefix(self.named_prefix)?;
        let names: Vec<&str> = tail.splitn(self.names + 1, '/').take(self.names).collect();
        if names.len() < self.names || names.iter().any(|name| name.is_empty()) {
            return None;
        }
        let len = names.iter().map(|name| name.len()).sum::<usize>() + self.names - 1;
        Some(&tail[len..])
    }
}

/// Maps every numeric-id URL in a `Link` value back to `request_path`, changing only the URL's path: the host, the
/// query, the `rel`s and their order are kept byte for byte. A URL is mapped only when `request_path` has the alias's
/// shape and ends with the same rest of the path; otherwise it's left as it is. The flag is whether any numeric-id
/// URL was seen, mapped or not: either way the input isn't what a fixture should hold.
fn rewrite_id_links(link: &str, request_path: &str) -> (String, bool) {
    let mut flagged = false;
    let rewritten = Rules::get().link_url.replace_all(link, |caps: &Captures| {
        let url = &caps[1];
        let (start, end) = url_path_span(url);
        let Some((alias, rest)) = IdAlias::split(&url[start..end]) else {
            return caps[0].to_owned();
        };
        flagged = true;
        if alias.rest_of(request_path) == Some(rest) {
            format!("<{}{request_path}{}>", &url[..start], &url[end..])
        } else {
            caps[0].to_owned()
        }
    });
    (rewritten.into_owned(), flagged)
}

/// The byte range of a URL's path: after `scheme://host`, up to the query or fragment. A URL with no scheme is taken
/// to start with its path.
fn url_path_span(url: &str) -> (usize, usize) {
    let start = url.find("://").map_or(0, |scheme| {
        let host = scheme + 3;
        url[host..].find('/').map_or(url.len(), |slash| host + slash)
    });
    let end = url[start..].find(['?', '#']).map_or(url.len(), |i| start + i);
    (start, end)
}

/// Image extensions that the email pattern mistakes for a TLD, as in `logo@2x.png`.
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "svg", "webp"];

/// Whether a match of the text email pattern should be replaced. Safe domains, SSH remotes (`git@github.com:o/r`)
/// and retina image names (`logo@2x.png`) are kept. Everything else is treated as an address: a false positive costs
/// a cosmetic diff, a false negative leaks one.
fn is_personal_email(candidate: &str) -> bool {
    let Some((local, domain)) = candidate.rsplit_once('@') else {
        return false;
    };
    let tld = domain.rsplit_once('.').map_or("", |(_, tld)| tld);
    !(SAFE_EMAIL_DOMAINS.iter().any(|safe| domain.eq_ignore_ascii_case(safe))
        || local == "git"
        || IMAGE_EXTENSIONS.iter().any(|ext| tld.eq_ignore_ascii_case(ext)))
}

/// Repo object keys that GitHub only returns to viewers with push or admin access. The scrubber drops them, so a
/// recording looks like what a signed-in user without push access gets, matching the pinned `permissions`.
const VIEWER_ONLY_KEYS: &[&str] = &[
    "security_and_analysis",
    "temp_clone_token",
    "allow_squash_merge",
    "allow_merge_commit",
    "allow_rebase_merge",
    "allow_auto_merge",
    "delete_branch_on_merge",
    "allow_update_branch",
    "use_squash_pr_title_as_default",
    "squash_merge_commit_message",
    "squash_merge_commit_title",
    "merge_commit_message",
    "merge_commit_title",
];

/// Pins a repo `permissions` object to what a signed-in user without push access has: `pull` only. Returns whether
/// anything changed. Key order is kept.
fn pin_read_only_permissions(map: &mut serde_json::Map<String, Value>) -> bool {
    let mut changed = false;
    for (key, flag) in map.iter_mut() {
        let want = Value::Bool(key == "pull");
        if matches!(flag, Value::Bool(_)) && *flag != want {
            *flag = want;
            changed = true;
        }
    }
    changed
}

/// Applies the field rules (emails, avatars and repo permissions) to `value` under `key`, returning the issue when
/// it changed.
fn replace_known_field(key: &str, value: &mut Value, pin_viewer: bool) -> Option<Issue> {
    if pin_viewer && key == "permissions" {
        let Value::Object(map) = value else { return None };
        return pin_read_only_permissions(map).then_some(Issue::Viewer);
    }
    let (issue, placeholder) = if EMAIL_KEYS.contains(&key) {
        (Issue::Email, PLACEHOLDER_EMAIL)
    } else if key == "avatar_url" {
        (Issue::Avatar, PLACEHOLDER_AVATAR)
    } else if key == "gravatar_id" {
        (Issue::Avatar, "")
    } else {
        return None;
    };
    match value {
        Value::String(s) if s != placeholder => {
            placeholder.clone_into(s);
            Some(issue)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Built at runtime so the repo never contains a string that trips secret scanning.
    pub(crate) fn fake_token() -> String {
        format!("gh{}_{}", 'p', "a".repeat(36))
    }

    fn entry() -> RecipeEntry {
        RecipeEntry::get("/repos/o/r")
    }

    fn json_response(body: &Value) -> RawResponse {
        RawResponse {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json; charset=utf-8".into())],
            body: body.to_string().into_bytes(),
        }
    }

    fn text_response(body: &str) -> RawResponse {
        RawResponse {
            status: 200,
            headers: vec![("Content-Type".into(), "text/plain; charset=utf-8".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn parses_gh_include_output() {
        let out =
            b"HTTP/1.1 404 Not Found\nContent-Type: application/json\r\nX-Github-Request-Id: 1:2\r\n\r\n{\"a\":1}\n";
        let resp = RawResponse::parse(out).unwrap();
        assert_eq!(resp.status, 404);
        assert_eq!(
            resp.headers,
            [
                ("Content-Type".to_owned(), "application/json".to_owned()),
                ("X-Github-Request-Id".to_owned(), "1:2".to_owned())
            ]
        );
        assert_eq!(resp.body, b"{\"a\":1}\n");

        let h2 = RawResponse::parse(b"HTTP/2.0 204 No Content\r\n\r\n").unwrap();
        assert_eq!(h2.status, 204);
        assert!(h2.body.is_empty());

        assert!(RawResponse::parse(b"{\"message\":\"Bad credentials\"}").is_err());
        assert!(RawResponse::parse(b"HTTP/2.0 abc\n\r\n").is_err());
    }

    #[test]
    fn token_in_json_body_aborts_with_its_pointer() {
        let body = json!({"items": [{"note": format!("token {}", fake_token())}]});
        let err = scrub(json_response(&body), &entry()).unwrap_err();
        assert_eq!(
            err.findings,
            [Finding {
                location: Location::Json("/items/0/note".into()),
                issue: Issue::Secret("GitHub token"),
            }]
        );
        assert!(err.to_string().contains(r#"JSON pointer "/items/0/note""#), "{err}");
    }

    #[test]
    fn token_in_text_body_aborts_with_its_offset() {
        let err = scrub(text_response(&format!("line\nT={}", fake_token())), &entry()).unwrap_err();
        assert_eq!(err.findings[0].location, Location::Byte(7));
    }

    #[test]
    fn token_in_a_dropped_header_is_dropped() {
        let mut resp = json_response(&json!({}));
        resp.headers.push(("X-Oauth-Token".into(), fake_token()));
        let out = scrub(resp, &entry()).unwrap();
        assert_eq!(out.headers.keys().collect::<Vec<_>>(), ["Content-Type"]);
    }

    #[test]
    fn other_secret_patterns_abort() {
        let secrets = [
            format!("github_{}_{}", "pat", "A".repeat(22)),
            format!("-----BEGIN RSA {} KEY-----", "PRIVATE"),
            format!("{}{}", "AKIA", "A".repeat(16)),
            format!("xox{}-{}", 'b', "1".repeat(10)),
        ];
        for secret in secrets {
            assert!(scrub(text_response(&secret), &entry()).is_err(), "{secret}");
        }
    }

    #[test]
    fn recorder_specific_repo_access_is_pinned_and_dropped() {
        let body = json!({
            "name": "r",
            "permissions": {"admin": true, "maintain": true, "push": true, "triage": true, "pull": true},
            "security_and_analysis": {"secret_scanning": {"status": "disabled"}},
            "default_branch": "main",
            "allow_auto_merge": true,
            "performed_via_github_app": {"permissions": {"issues": "write"}},
        });
        let mut got = body.clone();
        let mut findings = Vec::new();
        Rules::get().scrub_json(&mut got, &mut String::new(), &mut findings, true);
        assert_eq!(
            got,
            json!({
                "name": "r",
                "permissions": {"admin": false, "maintain": false, "push": false, "triage": false, "pull": true},
                "default_branch": "main",
                "performed_via_github_app": {"permissions": {"issues": "write"}},
            })
        );
        let keys: Vec<_> = got.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["name", "permissions", "default_branch", "performed_via_github_app"]
        );
        assert_eq!(
            findings,
            [
                Finding {
                    location: Location::Json("/security_and_analysis".into()),
                    issue: Issue::Viewer,
                },
                Finding {
                    location: Location::Json("/allow_auto_merge".into()),
                    issue: Issue::Viewer,
                },
                Finding {
                    location: Location::Json("/permissions".into()),
                    issue: Issue::Viewer,
                },
            ]
        );
        assert!(!findings[0].issue.aborts());

        let mut again = Vec::new();
        Rules::get().scrub_json(&mut got.clone(), &mut String::new(), &mut again, true);
        assert!(again.is_empty(), "{again:?}");
        assert_eq!(
            scrub(json_response(&body), &entry()).unwrap().body,
            ScrubbedBody::Json(got)
        );
    }

    #[test]
    fn emails_in_json_are_replaced_and_null_stays() {
        let body = json!({
            "commit": {"author": {"email": "a@corp.test"}, "committer_email": "b@corp.test"},
            "author_email": null,
            "avatar_url": "https://avatars.githubusercontent.com/u/42?v=4",
            "gravatar_id": "abc",
            "text": "kept a@corp.test",
        });
        let out = scrub(json_response(&body), &entry()).unwrap();
        assert_eq!(
            out.body,
            ScrubbedBody::Json(json!({
                "commit": {"author": {"email": PLACEHOLDER_EMAIL}, "committer_email": PLACEHOLDER_EMAIL},
                "author_email": null,
                "avatar_url": PLACEHOLDER_AVATAR,
                "gravatar_id": "",
                "text": "kept a@corp.test",
            }))
        );
        // Key order is preserved.
        let ScrubbedBody::Json(value) = out.body else {
            unreachable!()
        };
        let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["commit", "author_email", "avatar_url", "gravatar_id", "text"]);
    }

    #[test]
    fn emails_in_a_diff_are_replaced() {
        let diff = "From: Jane <jane@corp.test>\n+ cc bot@users.noreply.github.com, x@example.com\n";
        let mut resp = text_response(diff);
        resp.headers = vec![("Content-Type".into(), "text/plain; charset=utf-8".into())];
        let out = scrub(resp, &RecipeEntry::get("/repos/o/r/pulls/1")).unwrap();
        assert_eq!(
            out.body,
            ScrubbedBody::Text(
                "From: Jane <user@example.com>\n+ cc bot@users.noreply.github.com, x@example.com\n".into()
            )
        );
    }

    #[test]
    fn ssh_remotes_and_retina_images_are_not_emails() {
        let readme =
            "clone git@github.com:gizmo385/gh-lazy.git\n![logo](logo@2x.png) ![x](a@3x.JPEG) b@c.svg\nme@corp.test\n";
        let out = scrub(text_response(readme), &entry()).unwrap();
        assert_eq!(
            out.body,
            ScrubbedBody::Text(readme.replace("me@corp.test", PLACEHOLDER_EMAIL))
        );
        assert!(is_personal_email("gitty@github.com"));
        assert!(is_personal_email("jane@png.io"));
    }

    #[test]
    fn private_true_aborts() {
        let body = json!([{"name": "r", "private": false}, {"name": "secret", "private": true}]);
        let err = scrub(json_response(&body), &entry()).unwrap_err();
        assert_eq!(
            err.findings,
            [Finding {
                location: Location::Json("/1".into()),
                issue: Issue::Private
            }]
        );
    }

    #[test]
    fn volatile_headers_are_dropped_and_link_is_kept() {
        let mut resp = json_response(&json!([]));
        for (k, v) in [
            ("Date", "Sun, 25 Oct 2026 04:03:39 GMT"),
            ("Etag", "\"abc\""),
            ("x-ratelimit-remaining", "14821"),
            ("X-Github-Request-Id", "2062:3FB230"),
            ("X-Oauth-Scopes", "repo"),
            ("Github-Authentication-Token-Expiration", "2026-10-25 09:59:02 UTC"),
            ("link", "<https://api.github.com/x?page=2>; rel=\"next\""),
            ("Location", "https://example.com/logs.zip"),
        ] {
            resp.headers.push((k.into(), v.into()));
        }
        let out = scrub(resp, &entry()).unwrap();
        assert_eq!(
            out.headers,
            BTreeMap::from([
                ("Content-Type".to_owned(), "application/json; charset=utf-8".to_owned()),
                (
                    "Link".to_owned(),
                    "<https://api.github.com/x?page=2>; rel=\"next\"".to_owned()
                ),
                ("Location".to_owned(), "https://example.com/logs.zip".to_owned()),
            ])
        );
    }

    #[test]
    fn a_rate_limited_response_keeps_its_rate_limit_headers() {
        let headers = [
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "4102444800"),
            ("retry-after", "60"),
            ("X-Github-Request-Id", "2062:3FB230"),
        ];
        for status in [403, 429] {
            let mut resp = json_response(&json!({"message": "API rate limit exceeded"}));
            resp.status = status;
            resp.headers
                .extend(headers.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
            let out = scrub(resp, &entry()).unwrap();
            assert_eq!(
                out.headers.keys().collect::<Vec<_>>(),
                [
                    "Content-Type",
                    "Retry-After",
                    "X-Ratelimit-Remaining",
                    "X-Ratelimit-Reset"
                ],
                "{status}"
            );
        }
    }

    fn link_response(link: &str) -> RawResponse {
        let mut resp = json_response(&json!([]));
        resp.headers.push(("Link".into(), link.into()));
        resp
    }

    fn id_link_finding() -> Finding {
        Finding {
            location: Location::Header("Link".into()),
            issue: Issue::IdLink,
        }
    }

    #[test]
    fn each_numeric_id_alias_maps_to_the_request_path() {
        for (link_path, request_path) in [
            ("/repositories/812868589/issues", "/repos/gizmo385/gh-lazy/issues"),
            ("/user/42/repos", "/users/gizmo385/repos"),
            ("/organizations/7/repos", "/orgs/acme/repos"),
            ("/repositories/1", "/repos/o/r"),
        ] {
            let link = format!("<https://api.github.com{link_path}?page=2>; rel=\"next\"");
            assert_eq!(
                rewrite_id_links(&link, request_path),
                (
                    format!("<https://api.github.com{request_path}?page=2>; rel=\"next\""),
                    true
                ),
                "{link_path}"
            );
        }
    }

    #[test]
    fn every_rel_is_rewritten_and_the_rest_is_kept() {
        let id = "https://api.github.com/repositories/2000070/issues";
        let named = "https://api.github.com/repos/parity-bot/lazy-fixture/issues";
        let header = |base: &str| {
            format!(
                "<{base}?state=all&page=1&per_page=30>; rel=\"prev\", <{base}?state=all&page=3&per_page=30&after=Y3V%3D>; \
                 rel=\"next\", <{base}?state=all&page=3&per_page=30>; rel=\"last\", <{base}?page=1>; rel=\"first\""
            )
        };
        let mut entry = entry();
        entry.path = "/repos/parity-bot/lazy-fixture/issues".into();
        let out = scrub(link_response(&header(id)), &entry).unwrap();
        assert_eq!(out.headers["Link"], header(named));
        assert_eq!(rewrite_id_links(&header(id), &entry.path), (header(named), true));
    }

    /// Base's issues page was fixed by hand (#151), so `record-fixtures --set base --check` stays clean only if the
    /// edit is exactly what the scrubber writes for GitHub's original header.
    #[test]
    fn base_issues_link_is_what_the_scrubber_writes() {
        let recorded = "<https://api.github.com/repositories/812868589/issues?page=2&per_page=30&state=all&\
                        after=Y3Vyc29yOnYyOpLPAAABoXDF_RDPAAAAAWbO-iA%3D>; rel=\"next\"";
        let path = crate::fixtures::repo_root()
            .join("parity/fixtures/base/repos/gizmo385/gh-lazy/issues/GET.page-1_per_page-30_state-all.json");
        let fixture: Fixture = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut entry = entry();
        entry.path.clone_from(&fixture.request.path);
        let out = scrub(link_response(recorded), &entry).unwrap();
        assert_eq!(out.headers["Link"], fixture.response.headers["Link"]);
    }

    #[test]
    fn a_correct_link_is_unchanged_with_no_finding() {
        let link = "<https://api.github.com/user/repos?page=2&per_page=30>; rel=\"next\", \
                    <https://api.github.com/repos/o/r/issues?page=3>; rel=\"last\"";
        assert_eq!(rewrite_id_links(link, "/user/repos"), (link.to_owned(), false));
        let fixture: Fixture = serde_json::from_value(json!({
            "version": 1,
            "request": {"method": "GET", "path": "/user/repos"},
            "response": {"status": 200, "headers": {"Link": link}}
        }))
        .unwrap();
        let findings = Rules::get().check_fixture(&fixture, None);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_mismatched_rest_is_left_alone_and_reported() {
        let link = "<https://api.github.com/repositories/9/pulls?page=2>; rel=\"next\"";
        for request_path in [
            "/repos/o/r/issues",
            "/users/o/pulls",
            "/repos/o",
            "/repos/o/r/issues/pulls",
        ] {
            assert_eq!(
                rewrite_id_links(link, request_path),
                (link.to_owned(), true),
                "{request_path}"
            );
        }
        // A rewritable URL next to an unmappable one: the first is mapped, the second kept.
        let mixed = "<https://api.github.com/repositories/9/issues?page=2>; rel=\"next\", \
                     <https://api.github.com/user/9/repos?page=1>; rel=\"first\"";
        assert_eq!(
            rewrite_id_links(mixed, "/repos/o/r/issues").0,
            "<https://api.github.com/repos/o/r/issues?page=2>; rel=\"next\", \
             <https://api.github.com/user/9/repos?page=1>; rel=\"first\""
        );
        let fixture: Fixture = serde_json::from_value(json!({
            "version": 1,
            "request": {"method": "GET", "path": "/repos/o/r/issues"},
            "response": {"status": 200, "headers": {"Link": link}}
        }))
        .unwrap();
        assert_eq!(Rules::get().check_fixture(&fixture, None), [id_link_finding()]);
        assert_eq!(
            id_link_finding().to_string(),
            "header Link: points at a numeric-id path (/repositories/<id>/...); should be the request's path"
        );
        assert!(!Issue::IdLink.aborts());
    }

    #[test]
    fn location_is_not_rewritten() {
        let mut resp = json_response(&json!({}));
        resp.headers.push((
            "Location".into(),
            "https://api.github.com/repositories/812868589/issues/1".into(),
        ));
        let out = scrub(resp, &RecipeEntry::get("/repos/gizmo385/gh-lazy/issues/1")).unwrap();
        assert_eq!(
            out.headers["Location"],
            "https://api.github.com/repositories/812868589/issues/1"
        );
    }

    #[test]
    fn non_utf8_body_aborts_unless_binary() {
        let resp = RawResponse {
            status: 200,
            headers: vec![("Content-Type".into(), "application/zip".into())],
            body: vec![b'P', b'K', 0xff, 0xfe],
        };
        let err = scrub(resp.clone(), &entry()).unwrap_err();
        assert_eq!(err.findings[0].issue, Issue::NotUtf8);

        let mut binary = entry();
        binary.binary = true;
        let out = scrub(resp, &binary).unwrap();
        assert_eq!(out.body, ScrubbedBody::Binary(vec![b'P', b'K', 0xff, 0xfe]));
    }

    #[test]
    fn json_that_does_not_parse_is_text() {
        let mut resp = text_response("not json");
        resp.headers = vec![("Content-Type".into(), "application/json".into())];
        let out = scrub(resp, &entry()).unwrap();
        assert_eq!(out.body, ScrubbedBody::Text("not json".into()));
    }

    #[test]
    fn check_reports_what_scrub_would_change() {
        let fixture: Fixture = serde_json::from_value(json!({
            "version": 1,
            "request": {"method": "GET", "path": "/x"},
            "response": {
                "status": 200,
                "headers": {"Etag": "x", "Link": "y"},
                "body": {"email": format!("someone@{}", "gmail.com"), "avatar_url": PLACEHOLDER_AVATAR}
            }
        }))
        .unwrap();
        let findings = Rules::get().check_fixture(&fixture, None);
        assert_eq!(
            findings,
            [
                Finding {
                    location: Location::Header("Etag".into()),
                    issue: Issue::Header
                },
                Finding {
                    location: Location::Json("/email".into()),
                    issue: Issue::Email
                },
            ]
        );
    }
}
