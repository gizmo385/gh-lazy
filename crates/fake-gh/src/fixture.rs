//! The fixture file format (v1) and request-key normalization.
//!
//! This module is the single source of truth for the format: fake-gh uses it to answer requests, and
//! `cargo xtask record-fixtures` (#68) uses it to write recorded responses. `parity/README.md` documents it.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The only fixture format version this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// The host that `gh api` talks to. Absolute endpoint URLs on this host are reduced to their path.
pub const API_ORIGIN: &str = "https://api.github.com";

/// One fixture file: a request matcher and the response to replay for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub version: u32,
    pub request: FixtureRequest,
    pub response: Response,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The `request` object of a fixture: the normalized key plus optional `body` and `when` matchers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureRequest {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: Query,
    #[serde(default, skip_serializing_if = "Accept::is_json")]
    pub accept: Accept,
    /// When present, the request body must equal this JSON value. `null` counts as absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// When present, the fixture only matches once the request log holds a served request for each entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<When>,
}

impl FixtureRequest {
    /// The method, path, query and accept that a request must equal for this fixture to be a candidate.
    pub fn key(&self) -> RequestKey {
        RequestKey {
            method: self.method.clone(),
            path: self.path.clone(),
            query: self.query.clone(),
            accept: self.accept.clone(),
        }
    }
}

/// The `when` condition of a fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// Every entry must match some earlier served request in the log.
    pub after: Vec<LoggedRequestMatcher>,
}

/// Matches a served request in the log. `query`, `accept` and `body` are only compared when given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggedRequestMatcher {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<Query>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<Accept>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

/// The response half of a fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub status: u16,
    /// Response headers. Names are Go-canonicalized when printed, whatever case the fixture uses.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// A JSON body, printed compactly on one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    /// A text body, printed byte for byte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    /// A sibling file holding the body, printed byte for byte. Relative to the fixture file's directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_file: Option<String>,
    /// How long fake-gh sleeps before printing, so scenarios can capture loading states.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub delay_ms: u64,
}

// serde's `skip_serializing_if` passes a reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// A query value: one string, or several when the key repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueryValue {
    One(String),
    Many(Vec<String>),
}

impl QueryValue {
    /// The values in request order.
    pub fn values(&self) -> &[String] {
        match self {
            QueryValue::One(v) => std::slice::from_ref(v),
            QueryValue::Many(vs) => vs,
        }
    }
}

/// A normalized query. A `BTreeMap`, so comparison ignores key order.
pub type Query = BTreeMap<String, QueryValue>;

/// Builds a [`Query`] from pairs in request order; a repeated key becomes [`QueryValue::Many`].
pub fn query_from_pairs<I, K, V>(pairs: I) -> Query
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let mut query = Query::new();
    for (k, v) in pairs {
        let v = v.into();
        match query.entry(k.into()) {
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(QueryValue::One(v));
            }
            std::collections::btree_map::Entry::Occupied(mut e) => {
                let slot = e.get_mut();
                *slot = match std::mem::replace(slot, QueryValue::Many(Vec::new())) {
                    QueryValue::One(first) => QueryValue::Many(vec![first, v]),
                    QueryValue::Many(mut vs) => {
                        vs.push(v);
                        QueryValue::Many(vs)
                    }
                };
            }
        }
    }
    query
}

/// A normalized `Accept` header.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum Accept {
    #[default]
    Json,
    Diff,
    Raw,
    Patch,
    /// Any other media type, kept literally.
    Other(String),
}

impl Accept {
    /// Normalizes an `Accept` header value (or a fixture's already-normalized name) to its short form.
    pub fn normalize(value: &str) -> Accept {
        let trimmed = value.trim();
        match trimmed.to_ascii_lowercase().as_str() {
            "" | "json" | "application/vnd.github+json" | "application/vnd.github.v3+json" | "application/json" => {
                Accept::Json
            }
            "diff" | "application/vnd.github.diff" | "application/vnd.github.v3.diff" => Accept::Diff,
            "raw"
            | "application/vnd.github.raw"
            | "application/vnd.github.v3.raw"
            | "application/vnd.github.raw+json"
            | "application/vnd.github.v3.raw+json" => Accept::Raw,
            "patch" | "application/vnd.github.patch" | "application/vnd.github.v3.patch" => Accept::Patch,
            _ => Accept::Other(trimmed.to_owned()),
        }
    }

    /// The full media type to send for this accept, as the recorder needs it.
    pub fn media_type(&self) -> &str {
        match self {
            Accept::Json => "application/vnd.github+json",
            Accept::Diff => "application/vnd.github.diff",
            Accept::Raw => "application/vnd.github.raw",
            Accept::Patch => "application/vnd.github.patch",
            Accept::Other(s) => s,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Accept::Json => "json",
            Accept::Diff => "diff",
            Accept::Raw => "raw",
            Accept::Patch => "patch",
            Accept::Other(s) => s,
        }
    }

    pub fn is_json(&self) -> bool {
        *self == Accept::Json
    }
}

impl From<String> for Accept {
    fn from(s: String) -> Self {
        Accept::normalize(&s)
    }
}

impl From<Accept> for String {
    fn from(a: Accept) -> Self {
        a.as_str().to_owned()
    }
}

impl fmt::Display for Accept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The part of a request that fixtures are keyed on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestKey {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: Query,
    #[serde(default)]
    pub accept: Accept,
}

/// Why an endpoint couldn't be turned into a [`RequestKey`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// An absolute URL on a host other than `api.github.com`. fake-gh treats this as a miss.
    #[error("endpoint is not on {API_ORIGIN}: {0}")]
    ForeignHost(String),
    /// gh's `{owner}`/`{repo}`/`{branch}` placeholders, which need a git repo to resolve. Unsupported.
    #[error("placeholders like {{owner}} are not supported: {0}")]
    Placeholder(String),
}

impl RequestKey {
    /// Normalizes an endpoint as `gh api` receives it.
    ///
    /// `url` can be a path (`/user/repos`, `user/repos`), a path with a query, or an absolute
    /// `https://api.github.com/...` URL. `extra_query` (from `-F`/`-f` on GET) is appended after the URL's own query.
    pub fn from_url(
        method: &str,
        url: &str,
        extra_query: &[(String, String)],
        accept: Accept,
    ) -> Result<Self, KeyError> {
        if ["{owner}", "{repo}", "{branch}"].iter().any(|p| url.contains(p)) {
            return Err(KeyError::Placeholder(url.to_owned()));
        }
        let rest = if let Some(rest) = strip_prefix_ignore_case(url, API_ORIGIN) {
            if !(rest.is_empty() || rest.starts_with(['/', '?'])) {
                return Err(KeyError::ForeignHost(url.to_owned()));
            }
            rest
        } else if url.contains("://") {
            return Err(KeyError::ForeignHost(url.to_owned()));
        } else {
            url
        };
        let rest = rest.split_once('#').map_or(rest, |(before, _)| before);
        let (raw_path, raw_query) = rest.split_once('?').unwrap_or((rest, ""));

        let mut pairs: Vec<(String, String)> = raw_query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap_or((p, ""));
                (percent_decode(k, true), percent_decode(v, true))
            })
            .collect();
        pairs.extend(extra_query.iter().cloned());

        Ok(RequestKey {
            method: method.to_ascii_uppercase(),
            path: normalize_path(raw_path),
            query: query_from_pairs(pairs),
            accept,
        })
    }
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// Percent-decodes a path, then ensures a leading `/` and no trailing `/`.
pub fn normalize_path(raw: &str) -> String {
    let decoded = percent_decode(raw, false);
    let trimmed = decoded.trim_end_matches('/');
    if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}

/// Decodes `%XX` escapes (and `+` as a space in query components). Invalid escapes are kept literally, and
/// decoded bytes that aren't UTF-8 are replaced, like Go's lenient handling in practice.
pub fn percent_decode(s: &str, plus_is_space: bool) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' if plus_is_space => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// The longest query slug kept verbatim in a file name.
const MAX_SLUG_LEN: usize = 80;

/// The canonical file name for a fixture with this key: `<METHOD>[.<accept>][.<query-slug>].json`.
///
/// Fixtures whose keys differ only in `body` or `when` get the same name; the writer then inserts `.<n>` before
/// `.json` (see [`numbered_file_name`]).
pub fn file_name_for(key: &RequestKey) -> String {
    let mut name = key.method.to_ascii_uppercase();
    if !key.accept.is_json() {
        name.push('.');
        name.push_str(&sanitize(key.accept.as_str()));
    }
    if !key.query.is_empty() {
        name.push('.');
        name.push_str(&query_slug(&key.query));
    }
    name.push_str(".json");
    name
}

/// [`file_name_for`] with a `.<n>` disambiguator. `n` of 1 or less gives the plain name.
pub fn numbered_file_name(key: &RequestKey, n: u32) -> String {
    let name = file_name_for(key);
    if n <= 1 {
        return name;
    }
    let stem = name.strip_suffix(".json").unwrap_or(&name);
    format!("{stem}.{n}.json")
}

/// The directory, relative to the set directory, that holds fixtures for this key. It mirrors the URL path, with
/// characters Windows forbids in file names percent-encoded.
pub fn dir_for(key: &RequestKey) -> PathBuf {
    key.path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(encode_segment)
        .collect()
}

/// Percent-encodes `<>:"\|?*` (and `%` itself, so the encoding is reversible) in one path segment.
pub fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for c in segment.chars() {
        if matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' | '%') || c.is_ascii_control() {
            let mut buf = [0; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `k-v` pairs in key order joined by `_`, each key and value with characters outside `[A-Za-z0-9._-]` replaced by
/// `-`. Longer than 80 characters, it's cut and a 4-hex-digit hash of the full query is appended.
fn query_slug(query: &Query) -> String {
    let mut slug = String::new();
    let mut canonical = String::new();
    for (k, v) in query {
        for value in v.values() {
            if !slug.is_empty() {
                slug.push('_');
                canonical.push('&');
            }
            let _ = write!(slug, "{}-{}", sanitize(k), sanitize(value));
            let _ = write!(canonical, "{k}={value}");
        }
    }
    if slug.len() <= MAX_SLUG_LEN {
        return slug;
    }
    // `sanitize` leaves only ASCII, so byte slicing is on a char boundary.
    format!("{}-{:04x}", &slug[..MAX_SLUG_LEN], hash16(canonical.as_bytes()))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// FNV-1a (32-bit) folded to 16 bits. Stable across platforms and Rust versions, unlike `DefaultHasher`.
fn hash16(bytes: &[u8]) -> u16 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in bytes {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    // Truncation is the point: fold the two halves together.
    #[allow(clippy::cast_possible_truncation)]
    let folded = ((h >> 16) ^ (h & 0xffff)) as u16;
    folded
}

/// Go's `textproto.CanonicalMIMEHeaderKey`: `x-ratelimit-limit` becomes `X-Ratelimit-Limit`. Names with characters
/// outside the token set are returned unchanged, as Go does.
pub fn canonical_header_name(name: &str) -> String {
    let valid = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
    if !valid {
        return name.to_owned();
    }
    let mut upper = true;
    name.chars()
        .map(|c| {
            let out = if upper {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            };
            upper = c == '-';
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(url: &str, extra: &[(&str, &str)]) -> RequestKey {
        let extra: Vec<(String, String)> = extra.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        RequestKey::from_url("get", url, &extra, Accept::Json).unwrap()
    }

    #[test]
    fn full_url_becomes_path() {
        let k = key("https://api.github.com/repos/o/r/issues/1/comments", &[]);
        assert_eq!(k.method, "GET");
        assert_eq!(k.path, "/repos/o/r/issues/1/comments");
        assert!(k.query.is_empty());
        assert_eq!(key("HTTPS://API.GITHUB.COM", &[]).path, "/");
    }

    #[test]
    fn foreign_hosts_and_placeholders_are_errors() {
        assert!(matches!(
            RequestKey::from_url("GET", "https://example.com/user", &[], Accept::Json),
            Err(KeyError::ForeignHost(_))
        ));
        assert!(matches!(
            RequestKey::from_url("GET", "https://api.github.com.evil.test/user", &[], Accept::Json),
            Err(KeyError::ForeignHost(_))
        ));
        assert!(matches!(
            RequestKey::from_url("GET", "/repos/{owner}/{repo}/pulls", &[], Accept::Json),
            Err(KeyError::Placeholder(_))
        ));
    }

    #[test]
    fn url_query_is_merged_with_fields() {
        let k = key(
            "https://api.github.com/user/repos?page=2&per_page=50",
            &[("type", "all"), ("page", "3")],
        );
        assert_eq!(k.path, "/user/repos");
        assert_eq!(
            k.query,
            query_from_pairs([("page", "2"), ("per_page", "50"), ("type", "all"), ("page", "3")])
        );
        assert_eq!(k.query["page"], QueryValue::Many(vec!["2".into(), "3".into()]));
        assert_eq!(k.query["type"], QueryValue::One("all".into()));
    }

    #[test]
    fn query_values_are_percent_decoded() {
        let k = key("/search/issues?q=repo%3Ao%2Fr+is%3Aopen", &[]);
        assert_eq!(k.query["q"], QueryValue::One("repo:o/r is:open".into()));
    }

    #[test]
    fn trailing_slash_and_leading_slash_are_normalized() {
        assert_eq!(key("/user/repos/", &[]).path, "/user/repos");
        assert_eq!(key("user/repos", &[]).path, "/user/repos");
        assert_eq!(key("/", &[]).path, "/");
    }

    #[test]
    fn path_is_percent_decoded() {
        assert_eq!(
            key("/repos/o/r/contents/a%20b%2Fc.md", &[]).path,
            "/repos/o/r/contents/a b/c.md"
        );
        // Not a valid escape: kept as-is.
        assert_eq!(key("/x/100%zz", &[]).path, "/x/100%zz");
        assert_eq!(key("/x/%4", &[]).path, "/x/%4");
    }

    #[test]
    fn accept_table() {
        let cases = [
            ("", Accept::Json),
            ("application/vnd.github+json", Accept::Json),
            ("application/vnd.github.v3+json", Accept::Json),
            ("application/json", Accept::Json),
            ("application/vnd.github.diff", Accept::Diff),
            ("application/vnd.github.v3.diff", Accept::Diff),
            ("application/vnd.github.raw", Accept::Raw),
            ("application/vnd.github.v3.raw", Accept::Raw),
            ("application/vnd.github.raw+json", Accept::Raw),
            ("application/vnd.github.patch", Accept::Patch),
            ("application/vnd.github.v3.patch", Accept::Patch),
            (" Application/VND.github.diff ", Accept::Diff),
            ("diff", Accept::Diff),
            (
                "application/vnd.github.star+json",
                Accept::Other("application/vnd.github.star+json".into()),
            ),
        ];
        for (input, want) in cases {
            assert_eq!(Accept::normalize(input), want, "{input:?}");
        }
    }

    #[test]
    fn file_names_follow_the_convention() {
        let k = key(
            "/user/repos",
            &[
                ("type", "all"),
                ("direction", "asc"),
                ("sort", "full_name"),
                ("page", "1"),
                ("per_page", "50"),
            ],
        );
        assert_eq!(
            file_name_for(&k),
            "GET.direction-asc_page-1_per_page-50_sort-full_name_type-all.json"
        );
        assert_eq!(dir_for(&k), PathBuf::from("user").join("repos"));

        let mut diff = key("/repos/o/r/pulls/12", &[]);
        diff.accept = Accept::Diff;
        assert_eq!(file_name_for(&diff), "GET.diff.json");
        assert_eq!(numbered_file_name(&diff, 2), "GET.diff.2.json");

        let mut other = key("/x", &[("q", "a b/c")]);
        other.accept = Accept::Other("application/vnd.github.star+json".into());
        assert_eq!(
            file_name_for(&other),
            "GET.application-vnd.github.star-json.q-a-b-c.json"
        );
    }

    #[test]
    fn long_query_slugs_are_cut_and_hashed() {
        let long = "x".repeat(100);
        let a = file_name_for(&key("/x", &[("q", &long)]));
        let b = file_name_for(&key("/x", &[("q", &format!("{long}y"))]));
        assert_eq!(a.len(), "GET.".len() + 80 + "-abcd".len() + ".json".len());
        assert_ne!(a, b, "the hash tells apart queries with the same prefix");
    }

    #[test]
    fn windows_unsafe_segments_are_encoded() {
        let k = key("/repos/o/r/contents/a:b*c", &[]);
        assert_eq!(
            dir_for(&k),
            ["repos", "o", "r", "contents", "a%3Ab%2Ac"].iter().collect::<PathBuf>()
        );
    }

    #[test]
    fn header_names_are_go_canonical() {
        assert_eq!(canonical_header_name("x-ratelimit-limit"), "X-Ratelimit-Limit");
        assert_eq!(canonical_header_name("ETAG"), "Etag");
        assert_eq!(canonical_header_name("content-type"), "Content-Type");
        assert_eq!(canonical_header_name("bad header"), "bad header");
    }

    #[test]
    fn fixture_round_trips_through_serde() {
        let text = r#"{
            "version": 1,
            "request": {"method": "GET", "path": "/user/repos", "query": {"page": "1", "k": ["a", "b"]},
                        "accept": "application/vnd.github.raw"},
            "response": {"status": 200, "headers": {"Link": "<x>; rel=\"next\""}, "body": [{"b": 1, "a": 2}]},
            "note": "hi"
        }"#;
        let f: Fixture = serde_json::from_str(text).unwrap();
        assert_eq!(f.request.accept, Accept::Raw);
        assert_eq!(f.request.query["k"], QueryValue::Many(vec!["a".into(), "b".into()]));
        // Key order inside bodies is preserved.
        assert_eq!(serde_json::to_string(&f.response.body).unwrap(), r#"[{"b":1,"a":2}]"#);
        let back: Fixture = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn unknown_fixture_fields_are_rejected() {
        let text = r#"{"version": 1, "request": {"method": "GET", "path": "/x", "qurey": {}},
                       "response": {"status": 200}}"#;
        assert!(serde_json::from_str::<Fixture>(text).is_err());
    }
}
