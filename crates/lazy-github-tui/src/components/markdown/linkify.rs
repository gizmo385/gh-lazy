//! Bare URLs, `www.` hosts and emails in text, found the way markdown-it's `gfm-like` preset finds them with
//! linkify-it-py 2.1.0 (default options: fuzzy links and emails on, fuzzy IPs off).
//!
//! linkify-it's patterns lean on lookarounds, which the `regex` crate doesn't have, so the hosts, ports, paths and
//! the checks after them are scanned by hand here, following the patterns in `linkify_it/ucre.py`; `regex` is used
//! for the Unicode character classes and the TLD lists. Positions are byte offsets into the text.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

/// linkify-it's TLDs for links without a scheme: its list, any two-letter country code, and punycode.
const TLDS: &str = "biz|com|edu|gov|net|org|pro|web|xxx|aero|asia|coop|info|museum|name|shop|рф|\
    a[cdefgilmnoqrstuwxz]|b[abdefghijmnorstvwyz]|c[acdfghiklmnoruvwxyz]|d[ejkmoz]|e[cegrstu]|f[ijkmor]|\
    g[abdefghilmnpqrstuwy]|h[kmnrtu]|i[delmnoqrst]|j[emop]|k[eghimnprwyz]|l[abcikrstuvy]|\
    m[acdeghklmnopqrstuvwxyz]|n[acefgilopruz]|om|p[aefghklmnrstwy]|qa|r[eosuw]|s[abcdeghijklmnortuvxyz]|\
    t[cdfghjklmnortvwz]|u[agksyz]|v[aceginu]|w[fs]|y[et]|z[amw]|xn--[a-z0-9\\-]{1,59}";

/// Compiles one of the fixed patterns below.
fn pattern(source: &str) -> Regex {
    // Fixed patterns, all compiled by `the_patterns_compile`, so they always compile.
    Regex::new(source).expect("valid pattern")
}

/// Whitespace, punctuation and control characters: linkify-it's `src_ZPCc`.
static ZPCC: LazyLock<Regex> = LazyLock::new(|| pattern(r"^[\p{Z}\p{P}\p{Cc}]$"));
/// Whitespace and control characters: `src_ZCc`.
static ZCC: LazyLock<Regex> = LazyLock::new(|| pattern(r"^[\p{Z}\p{Cc}]$"));
/// A whole label that is a TLD, any case.
static TLD: LazyLock<Regex> = LazyLock::new(|| pattern(&format!("(?i)^(?:{TLDS})$")));
/// A punycode label, any case: `src_xn`.
static XN: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?i)^xn--[a-z0-9\-]{1,59}$"));
/// linkify-it's quick test for a link without a scheme (`host_fuzzy_test`): without one before the scheme link
/// found, it doesn't look for one.
static FUZZY_TEST: LazyLock<Regex> = LazyLock::new(|| {
    pattern(&format!(
        r"(?i)localhost|www\.|\.\d{{1,3}}\.|\.(?:{TLDS})(?:[\p{{Z}}\p{{P}}\p{{Cc}}>]|$)"
    ))
});

/// A link found in text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Found {
    /// The bytes of the text it covers, which are shown as its text.
    pub(super) range: Range<usize>,
    /// Where it goes: the text, after `http://` for a host without a scheme and `mailto:` for a bare email.
    pub(super) url: String,
}

/// Every link in `text`, in order: linkify-it's `match`, which finds the first link and looks again after it.
pub(super) fn links(text: &str) -> Vec<Found> {
    let mut found = Vec::new();
    let mut shift = 0;
    while let Some((range, kind)) = first(&text[shift..]) {
        let range = range.start + shift..range.end + shift;
        let raw = &text[range.clone()];
        let url = match kind {
            Kind::Scheme => raw.to_owned(),
            Kind::Fuzzy => format!("http://{raw}"),
            Kind::Email => format!("mailto:{raw}"),
        };
        shift = range.end;
        found.push(Found { range, url });
    }
    found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `http:`, `https:`, `ftp:`, `mailto:` or `//`.
    Scheme,
    /// A host with a known TLD and no scheme.
    Fuzzy,
    /// An email without `mailto:`.
    Email,
}

/// linkify-it's `test`: the first link in `text`, from a scheme, a fuzzy host or an email, whichever starts first
/// (a longer email wins a tie).
fn first(text: &str) -> Option<(Range<usize>, Kind)> {
    let mut best = scheme_link(text).map(|range| (range, Kind::Scheme));
    let gate = FUZZY_TEST.find(text).map(|m| m.start());
    if let Some(gate) = gate
        && best.as_ref().is_none_or(|(range, _)| gate < range.start)
        && let Some(range) = fuzzy_link(text)
        && best.as_ref().is_none_or(|(found, _)| range.start < found.start)
    {
        best = Some((range, Kind::Fuzzy));
    }
    if text.contains('@')
        && let Some(range) = fuzzy_email(text)
        && best
            .as_ref()
            .is_none_or(|(found, _)| range.start < found.start || (range.start == found.start && range.end > found.end))
    {
        best = Some((range, Kind::Email));
    }
    best
}

fn char_at(text: &str, at: usize) -> Option<char> {
    text.get(at..).and_then(|rest| rest.chars().next())
}

fn char_before(text: &str, at: usize) -> Option<char> {
    text.get(..at).and_then(|head| head.chars().next_back())
}

fn matches_char(class: &Regex, c: char) -> bool {
    let mut buf = [0; 4];
    class.is_match(c.encode_utf8(&mut buf))
}

fn is_zpcc(c: char) -> bool {
    matches_char(&ZPCC, c)
}

fn is_zcc(c: char) -> bool {
    matches_char(&ZCC, c)
}

/// `TEXT_SEPARATORS`: characters that always end a link.
fn is_separator(c: char) -> bool {
    matches!(c, '<' | '>' | '｜')
}

/// `src_pseudo_letter`: anything that isn't whitespace, punctuation, a control character or a separator. Symbols
/// such as `$`, `+`, `=` and `|` count.
fn is_letter(c: char) -> bool {
    !is_zpcc(c) && !is_separator(c)
}

/// The first scheme link: linkify-it's `schema_search` loop over `http:`, `https:`, `ftp:`, `//` and `mailto:`,
/// each validated at its position.
fn scheme_link(text: &str) -> Option<Range<usize>> {
    const SCHEMES: [&str; 5] = ["http:", "https:", "ftp:", "//", "mailto:"];
    let mut at = 0;
    while at < text.len() {
        let scheme = SCHEMES.iter().find(|scheme| {
            text.get(at..at + scheme.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(scheme))
        });
        let Some(scheme) = scheme.filter(|scheme| scheme_may_start(text, at, scheme)) else {
            at += char_at(text, at).map_or(1, char::len_utf8);
            continue;
        };
        let after = at + scheme.len();
        let length = match *scheme {
            "//" => double_slash(text, after),
            "mailto:" => mailto(&text[after..]),
            _ => http(&text[after..]),
        };
        if let Some(length) = length.filter(|&length| length > 0) {
            let mut end = after + length;
            if *scheme != "//" && *scheme != "mailto:" {
                // markdown-it's inline linkify rule finds these first, and drops a trailing `*` (emphasis).
                end = at + text[at..end].trim_end_matches('*').len();
            }
            return Some(at..end);
        }
        // Searching resumes after the scheme, as Python's `finditer` does.
        at = after;
    }
    None
}

/// Whether a scheme at `at` can start a link. linkify-it wants the text's start, a separator, or whitespace,
/// punctuation or a control character other than `_` before it. markdown-it's inline rule, which finds
/// `http:`, `https:` and `ftp:` links first, wants anything but an ASCII letter or digit, `.`, `+` or `-`.
fn scheme_may_start(text: &str, at: usize, scheme: &str) -> bool {
    let Some(before) = char_before(text, at) else {
        return true;
    };
    let linkify_it = is_separator(before) || (is_zpcc(before) && before != '_');
    let inline_rule = scheme.ends_with("p:") && !(before.is_ascii_alphanumeric() || matches!(before, '.' | '+' | '-'));
    linkify_it || inline_rule
}

/// `http:`'s validator, on the text after the scheme: `//`, then an optional `user:pass@`, a host, a port, and a
/// path. The length matched, or `None`.
fn http(tail: &str) -> Option<usize> {
    let rest = tail.strip_prefix("//")?;
    let end = with_auth(rest, |at| host_port(rest, at, Host::Any))?;
    Some(2 + path(rest, end))
}

/// `//`'s validator: an optional `user:pass@`, then `localhost` or a host with a dot, a port, and a path. Not after
/// a `:` or a `/`.
fn double_slash(text: &str, after: usize) -> Option<usize> {
    let rest = &text[after..];
    let end = with_auth(rest, |at| {
        let local = rest
            .get(at..at + 9)
            .filter(|s| s.eq_ignore_ascii_case("localhost"))
            .and_then(|_| port_and_terminator(rest, at + 9));
        local.or_else(|| host_port(rest, at, Host::Dotted))
    })?;
    let before = after.checked_sub(3).and_then(|at| char_at(text, at));
    if matches!(before, Some(':' | '/')) {
        return None;
    }
    Some(path(rest, end))
}

/// `mailto:`'s validator: an email name, `@` and a host.
fn mailto(tail: &str) -> Option<usize> {
    let at = email_name(tail, 0)?;
    host_end(tail, at + 1, Host::Any).filter(|&end| terminates(tail, end))
}

/// Runs `host` from where the host starts, after a `user:pass@` if there is one and it works, else from the start.
fn with_auth(text: &str, host: impl Fn(usize) -> Option<usize>) -> Option<usize> {
    let auth = text
        .char_indices()
        .find(|&(_, c)| is_zcc(c) || matches!(c, '@' | '/' | '[' | ']' | '(' | ')'))
        .filter(|&(at, c)| c == '@' && at > 0)
        .map(|(at, _)| at + 1);
    auth.and_then(&host).or_else(|| host(0))
}

/// The first fuzzy link: a host with a known TLD, not right after `.`, `:`, `/`, `-`, `_` or `@`, then a port and a
/// path (`link_no_ip_fuzzy`).
fn fuzzy_link(text: &str) -> Option<Range<usize>> {
    text.char_indices().find_map(|(at, c)| {
        let prefix_ok = char_before(text, at).is_none_or(|before| {
            !matches!(before, '.' | ':' | '/' | '-' | '_' | '@')
                && (matches!(before, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '｜') || is_zpcc(before))
        });
        if !prefix_ok || matches!(c, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '｜') {
            return None;
        }
        let end = host_port(text, at, Host::Tld)?;
        Some(at..path(text, end))
    })
}

/// The first fuzzy email: a name after the text's start, a separator, `"`, `(` or whitespace, then `@` and a host
/// with a known TLD or an IPv4 address (`email_fuzzy`).
fn fuzzy_email(text: &str) -> Option<Range<usize>> {
    text.char_indices().find_map(|(at, _)| {
        let prefix_ok = char_before(text, at)
            .is_none_or(|before| is_separator(before) || matches!(before, '"' | '(') || is_zcc(before));
        if !prefix_ok {
            return None;
        }
        let sign = email_name(text, at)?;
        let host = sign + 1;
        let end = ipv4(text, host)
            .filter(|&end| terminates(text, end))
            .or_else(|| host_end(text, host, Host::Tld).filter(|&end| terminates(text, end)))?;
        Some(at..end)
    })
}

/// `src_email_name` from `at`, which must be followed by `@`: where the `@` is.
fn email_name(text: &str, at: usize) -> Option<usize> {
    let name_char = |c: char| c.is_ascii_alphanumeric() || "-:&=+$,._\"".contains(c);
    let first = char_at(text, at)?;
    if !name_char(first) || first == '"' {
        return None;
    }
    let len = text[at..].find(|c: char| !name_char(c))?;
    (char_at(text, at + len) == Some('@')).then_some(at + len)
}

/// Four dot-separated numbers up to 255 from `at`: `src_ip4`. Where it ends.
fn ipv4(text: &str, at: usize) -> Option<usize> {
    let mut end = at;
    for index in 0..4 {
        if index > 0 {
            end = if char_at(text, end) == Some('.') {
                end + 1
            } else {
                return None;
            };
        }
        let digits = text[end..]
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len() - end);
        let octet = &text[end..end + digits];
        if digits == 0 || digits > 3 || octet.parse::<u16>().ok()? > 255 {
            return None;
        }
        end += digits;
    }
    Some(end)
}

/// What a host must look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Host {
    /// Any domain (`src_host`).
    Any,
    /// At least two labels, the last a root label without hyphens (`//` links).
    Dotted,
    /// At least two labels, the last a known TLD (fuzzy links and emails).
    Tld,
}

/// The labels of a dotted host from `at` (each a run of letters and hyphens), checked as `kind` wants: where the
/// host ends. Only the whole run of labels can be the host: ending it earlier leaves a `.` and a letter after it,
/// which no host may be followed by.
fn host_end(text: &str, at: usize, kind: Host) -> Option<usize> {
    let mut labels = Vec::new();
    let mut start = at;
    loop {
        let len = text[start..]
            .find(|c: char| !(is_letter(c) || c == '-'))
            .unwrap_or(text.len() - start);
        if len == 0 {
            break;
        }
        labels.push(&text[start..start + len]);
        let end = start + len;
        let next = char_at(text, end + 1);
        if char_at(text, end) == Some('.') && next.is_some_and(|c| is_letter(c) || c == '-') {
            start = end + 1;
        } else {
            break;
        }
    }
    let (last, init) = labels.split_last()?;
    if !init.iter().all(|label| is_domain(label)) {
        return None;
    }
    let last_ok = match kind {
        Host::Any => is_domain(last),
        Host::Dotted => {
            !init.is_empty() && (XN.is_match(last) || (last.chars().count() <= 63 && last.chars().all(is_letter)))
        }
        Host::Tld => !init.is_empty() && TLD.is_match(last),
    };
    last_ok.then(|| start + last.len())
}

/// `src_domain`: punycode, or 1 to 63 letters and hyphens that start and end with a letter.
fn is_domain(label: &str) -> bool {
    if XN.is_match(label) {
        return true;
    }
    let count = label.chars().count();
    let ends = |c: Option<char>| c.is_some_and(is_letter);
    (1..=63).contains(&count) && ends(label.chars().next()) && ends(label.chars().next_back())
}

/// A host from `at`, then an optional port, then a valid end: where it all ends.
fn host_port(text: &str, at: usize, kind: Host) -> Option<usize> {
    let end = host_end(text, at, kind)?;
    port_and_terminator(text, end)
}

/// An optional `:port` (0 to 65535, as `src_port` allows it) at `at`, then the host terminator. Where it ends.
fn port_and_terminator(text: &str, at: usize) -> Option<usize> {
    if char_at(text, at) == Some(':') {
        let digits = text[at + 1..]
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len() - at - 1);
        let port = &text[at + 1..at + 1 + digits];
        let valid = match port.len() {
            1..=4 => true,
            5 => port.starts_with(['1', '2', '3', '4', '5']) || port.parse::<u32>().is_ok_and(|p| p <= 65535),
            _ => false,
        };
        if valid && terminates(text, at + 1 + digits) {
            return Some(at + 1 + digits);
        }
    }
    terminates(text, at).then_some(at)
}

/// `src_host_terminator`: what may follow a host. The end, a separator, or whitespace or punctuation, except `-`,
/// `_`, `:` and a digit, `.-`, and a `.` followed by anything but the end, whitespace or punctuation.
fn terminates(text: &str, at: usize) -> bool {
    let Some(c) = char_at(text, at) else {
        return true;
    };
    if !(is_separator(c) || is_zpcc(c)) {
        return false;
    }
    let next = char_at(text, at + c.len_utf8());
    match c {
        '-' | '_' => false,
        ':' => !next.is_some_and(|n| n.is_ascii_digit()),
        '.' => next.is_none_or(|n| n != '-' && is_zpcc(n)),
        _ => true,
    }
}

/// `src_path` from `at`: a `/`, `?` or `#` and what may follow it, or a lone `/`. Where it ends; `at` if there's no
/// path.
fn path(text: &str, at: usize) -> usize {
    let Some(first) = char_at(text, at) else {
        return at;
    };
    if matches!(first, '/' | '?' | '#') {
        let mut end = at + 1;
        while let Some(step) = path_step(text, end) {
            end += step;
        }
        if end > at + 1 {
            return end;
        }
    }
    if first == '/' { at + 1 } else { at }
}

/// One of `src_path`'s alternatives at `at`, in its order: how many bytes it takes, or `None` when none applies.
fn path_step(text: &str, at: usize) -> Option<usize> {
    let c = char_at(text, at)?;
    let rest = &text[at + c.len_utf8()..];
    let next = rest.chars().next();
    let usable = |c: char| !is_zcc(c) && !is_separator(c);
    // Ends a path piece: the end, whitespace or a control character.
    let at_end = |n: Option<char>| n.is_none_or(is_zcc);
    // A bracketed group with no whitespace inside: its length, or `None` if it doesn't close.
    let group = |close: char, at_least_one: bool| {
        let inner = rest.find(|n: char| is_zcc(n) || n == close)?;
        (char_at(rest, inner) == Some(close) && (inner > 0 || !at_least_one)).then_some(1 + inner + close.len_utf8())
    };
    if usable(c) && !"()[]{}.,\"'?!-;".contains(c) {
        return Some(c.len_utf8());
    }
    match c {
        '[' => group(']', false),
        '(' => group(')', false),
        '{' => group('}', false),
        '"' => group('"', true),
        '\'' => group('\'', true).or_else(|| next.filter(|&n| is_letter(n) || n == '-').map(|_| 1)),
        '.' => {
            let dots = 1 + rest.find(|n: char| n != '.').unwrap_or(rest.len());
            let after = char_at(text, at + dots);
            if dots >= 2 && after.is_some_and(|n| n.is_ascii_alphanumeric() || "%/&".contains(n)) {
                Some(dots + 1)
            } else if !at_end(next) && next != Some('.') {
                Some(1)
            } else {
                None
            }
        }
        '-' => Some(1 + rest.find(|n: char| n != '-').unwrap_or(rest.len())),
        ',' | ';' => (!at_end(next)).then_some(1),
        '!' => {
            // `\!+(?!ZCc|[!]|$)`: the whole run of `!`, if something other than an end follows it.
            let run = 1 + rest.find(|n: char| n != '!').unwrap_or(rest.len());
            (!at_end(char_at(text, at + run))).then_some(run)
        }
        '?' => (!at_end(next) && next != Some('?')).then_some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The links in `text` as (text, target) pairs.
    fn find(text: &str) -> Vec<(&str, String)> {
        links(text)
            .into_iter()
            .map(|found| (&text[found.range], found.url))
            .collect()
    }

    /// Every case from #218's spec, and more, each as `MarkdownIt("gfm-like").parse` links it in Python.
    #[rstest::rstest]
    #[case("https://example.com/a_(b)", &[("https://example.com/a_(b)", "https://example.com/a_(b)")])]
    #[case("see https://example.com.", &[("https://example.com", "https://example.com")])]
    #[case("//example.com/x", &[("//example.com/x", "//example.com/x")])]
    #[case("ftp://a.b/c", &[("ftp://a.b/c", "ftp://a.b/c")])]
    #[case("www.github.com", &[("www.github.com", "http://www.github.com")])]
    #[case("visit example.com now", &[("example.com", "http://example.com")])]
    #[case("foo.io", &[("foo.io", "http://foo.io")])]
    #[case("a.b.example.io/path?x=1#y", &[("a.b.example.io/path?x=1#y", "http://a.b.example.io/path?x=1#y")])]
    #[case("see README.md here", &[("README.md", "http://README.md")])]
    #[case("mail foo@bar.com", &[("foo@bar.com", "mailto:foo@bar.com")])]
    #[case("config.json", &[])]
    #[case("v1.2", &[])]
    #[case("x.dev", &[])]
    #[case("x@y", &[])]
    #[case("skype:me", &[])]
    #[case("1.2.3.4", &[])]
    #[case("HTTPS://EXAMPLE.COM/A", &[("HTTPS://EXAMPLE.COM/A", "HTTPS://EXAMPLE.COM/A")])]
    #[case("http://localhost:8080/x?y=1", &[("http://localhost:8080/x?y=1", "http://localhost:8080/x?y=1")])]
    #[case("http://a.com:99999/x", &[])]
    #[case("example.com:8080/path", &[("example.com:8080/path", "http://example.com:8080/path")])]
    #[case("example.com:abc", &[("example.com", "http://example.com")])]
    #[case("(see https://example.com/foo)", &[("https://example.com/foo", "https://example.com/foo")])]
    #[case("\"https://example.com/q\"", &[("https://example.com/q", "https://example.com/q")])]
    #[case("https://example.com/a,b,c, and more", &[("https://example.com/a,b,c", "https://example.com/a,b,c")])]
    #[case("https://example.com/a!!! wow", &[("https://example.com/a", "https://example.com/a")])]
    #[case("https://example.com/a?b? end", &[("https://example.com/a?b", "https://example.com/a?b")])]
    #[case("https://example.com/a... more", &[("https://example.com/a", "https://example.com/a")])]
    #[case("https://example.com/a...b", &[("https://example.com/a...b", "https://example.com/a...b")])]
    #[case("https://example.com/a--b-", &[("https://example.com/a--b-", "https://example.com/a--b-")])]
    #[case("https://example.com/'x' y", &[("https://example.com/'x'", "https://example.com/'x'")])]
    #[case("https://example.com/it's", &[("https://example.com/it's", "https://example.com/it's")])]
    #[case(
        "https://user:pass@example.com/x",
        &[("https://user:pass@example.com/x", "https://user:pass@example.com/x")]
    )]
    #[case("_http://a.com", &[("http://a.com", "http://a.com")])]
    #[case("xhttp://a.com", &[])]
    #[case("x.https://a.com", &[("https://a.com", "https://a.com")])]
    #[case("http://a.com/b*", &[("http://a.com/b", "http://a.com/b")])]
    #[case("foo_bar.com", &[])]
    #[case("foo-bar.com", &[("foo-bar.com", "http://foo-bar.com")])]
    #[case("-foo.com", &[])]
    #[case("foo.com-", &[])]
    #[case("foo.com_", &[])]
    #[case("foo.co.uk/x", &[("foo.co.uk/x", "http://foo.co.uk/x")])]
    #[case("example.community", &[])]
    #[case("EXAMPLE.COM", &[("EXAMPLE.COM", "http://EXAMPLE.COM")])]
    #[case("exa$mple.com", &[("exa$mple.com", "http://exa$mple.com")])]
    #[case("$example.com", &[("example.com", "http://example.com")])]
    #[case("a.b.c.d.e.f.org", &[("a.b.c.d.e.f.org", "http://a.b.c.d.e.f.org")])]
    #[case("x.y", &[])]
    #[case("http://x", &[("http://x", "http://x")])]
    #[case("http://1.2.3.4/x", &[("http://1.2.3.4/x", "http://1.2.3.4/x")])]
    #[case("a@1.2.3.4", &[("a@1.2.3.4", "mailto:a@1.2.3.4")])]
    #[case("foo.bar@baz.org", &[("foo.bar@baz.org", "mailto:foo.bar@baz.org")])]
    #[case("(a@b.com)", &[("a@b.com", "mailto:a@b.com")])]
    #[case("a@b.com.", &[("a@b.com", "mailto:a@b.com")])]
    #[case("mailto:foo@bar.com", &[("mailto:foo@bar.com", "mailto:foo@bar.com")])]
    #[case("mailto:foo", &[])]
    #[case("www.x", &[])]
    #[case(
        "www.example.com/path/to/file.html#frag",
        &[("www.example.com/path/to/file.html#frag", "http://www.example.com/path/to/file.html#frag")]
    )]
    #[case("example.com/~user/", &[("example.com/~user/", "http://example.com/~user/")])]
    #[case(
        "https://en.wikipedia.org/wiki/Foo_(bar)_baz",
        &[("https://en.wikipedia.org/wiki/Foo_(bar)_baz", "https://en.wikipedia.org/wiki/Foo_(bar)_baz")]
    )]
    #[case(
        "see http://a.com and http://b.com, ok",
        &[("http://a.com", "http://a.com"), ("http://b.com", "http://b.com")]
    )]
    #[case("a.com/b.c.d/e", &[("a.com/b.c.d/e", "http://a.com/b.c.d/e")])]
    #[case("https://example.com/a;b; end", &[("https://example.com/a;b", "https://example.com/a;b")])]
    #[case("https://x.com/<y", &[("https://x.com/", "https://x.com/")])]
    #[case("a.com>b", &[("a.com", "http://a.com")])]
    #[case("//a.b", &[("//a.b", "//a.b")])]
    #[case("://a.com", &[])]
    #[case("x//a.com", &[])]
    #[case(":www.a.com", &[])]
    #[case(".www.a.com", &[])]
    #[case("@www.a.com", &[])]
    #[case("мой.рф", &[("мой.рф", "http://мой.рф")])]
    #[case("\"quoted\"@x.com", &[("quoted\"@x.com", "mailto:quoted\"@x.com")])]
    fn links_match_linkify_it(#[case] text: &str, #[case] expected: &[(&str, &str)]) {
        let expected: Vec<(&str, String)> = expected.iter().map(|&(t, u)| (t, u.to_owned())).collect();
        assert_eq!(find(text), expected);
    }

    #[test]
    fn the_patterns_compile() {
        for class in [&ZPCC, &ZCC, &TLD, &XN, &FUZZY_TEST] {
            LazyLock::force(class);
        }
    }

    #[test]
    fn odd_text_does_not_panic() {
        for text in [
            "",
            "@",
            ".",
            "a.",
            "a..com",
            "http://",
            "//",
            "x@",
            "@a.com",
            "é.com",
            "a.com:",
            "a.com:6",
            "😀.com 😀@😀.com",
        ] {
            links(text);
        }
    }
}
