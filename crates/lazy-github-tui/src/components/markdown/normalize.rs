//! Link targets and link text normalized as markdown-it-py 4.0.0 does (`common/normalize_url.py`): every link and
//! image target goes through [`normalize_link`], the text of autolinks and linkified URLs through
//! [`normalize_link_text`], and a target that fails [`validate_link`] isn't a link.
//!
//! Both parse the URL with a port of mdurl 0.1.2's `parse` (Node's legacy `url.parse`, not WHATWG parsing, which the
//! `url` crate does and which differs), recode the hostname with punycode, and format it back. The punycode is in
//! house: markdown-it keeps a label's case (`Bücher` becomes `xn--Bcher-kva`), which UTS 46 (`idna`) doesn't.

use super::parse::is_space;

/// markdown-it's `normalizeLink`: the hostname of an `http:`, `https:` or `mailto:` URL, or one without a scheme, to
/// punycode, then everything but ASCII letters, digits, `;/?:@&=+$,-_.!~*'()#` and valid `%XX` escapes
/// percent-encoded.
pub(super) fn normalize_link(url: &str) -> String {
    let mut parsed = parse(url);
    if recodes_hostname(&parsed) {
        parsed.hostname = to_ascii(&parsed.hostname);
    }
    encode(&parsed.format())
}

/// markdown-it's `normalizeLinkText`: the hostname of an `http:`, `https:` or `mailto:` URL, or one without a scheme,
/// from punycode to Unicode, then the `%XX` escapes decoded, except those of `;/?:@&=+$,#%`.
pub(super) fn normalize_link_text(url: &str) -> String {
    let mut parsed = parse(url);
    if recodes_hostname(&parsed)
        && let Some(hostname) = to_unicode(&parsed.hostname)
    {
        parsed.hostname = hostname;
    }
    decode(&parsed.format(), ";/?:@&=+$,#%")
}

/// markdown-it's `validateLink`, on a normalized target: anything but `vbscript:`, `javascript:`, `file:` and `data:`
/// (any case), except `data:` GIF, PNG, JPEG and WebP images.
pub(super) fn validate_link(url: &str) -> bool {
    let url = url.trim_matches(is_space).to_lowercase();
    let bad = ["vbscript:", "javascript:", "file:", "data:"]
        .iter()
        .any(|scheme| url.starts_with(scheme));
    !bad || ["gif", "png", "jpeg", "webp"]
        .iter()
        .any(|kind| url.starts_with(&format!("data:image/{kind};")))
}

/// Whether markdown-it recodes the hostname: there is one, and the scheme (as written, so `HTTP:` isn't one) is
/// `http:`, `https:`, `mailto:` or missing.
fn recodes_hostname(url: &Url) -> bool {
    !url.hostname.is_empty()
        && url
            .protocol
            .as_deref()
            .is_none_or(|p| ["http:", "https:", "mailto:"].contains(&p))
}

/// mdurl's `URL`: the parts of a URL, as written. An empty part is missing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Url {
    /// The scheme and its `:`.
    protocol: Option<String>,
    /// Whether `//` followed the scheme.
    slashes: bool,
    auth: String,
    port: String,
    /// Without an IPv6 address's brackets.
    hostname: String,
    /// The fragment and its `#`.
    hash: String,
    /// The query and its `?`.
    search: String,
    pathname: String,
}

impl Url {
    /// mdurl's `format`.
    fn format(&self) -> String {
        let mut out = String::new();
        out.push_str(self.protocol.as_deref().unwrap_or_default());
        if self.slashes {
            out.push_str("//");
        }
        if !self.auth.is_empty() {
            out.push_str(&self.auth);
            out.push('@');
        }
        if self.hostname.contains(':') {
            out.push('[');
            out.push_str(&self.hostname);
            out.push(']');
        } else {
            out.push_str(&self.hostname);
        }
        if !self.port.is_empty() {
            out.push(':');
            out.push_str(&self.port);
        }
        out.push_str(&self.pathname);
        out.push_str(&self.search);
        out.push_str(&self.hash);
        out
    }
}

/// Characters that end a hostname (mdurl's `NON_HOST_CHARS`).
const NON_HOST_CHARS: [char; 19] = [
    '%', '/', '?', ';', '#', '\'', '{', '}', '|', '\\', '^', '`', '<', '>', '"', ' ', '\r', '\n', '\t',
];

/// The longest a hostname may be, in characters; a longer one is dropped.
const HOSTNAME_MAX_LEN: usize = 255;

/// A character of a scheme: `[a-z0-9.+-]` as Python matches it ignoring case, which also takes `İ`, `ı`, `ſ` and the
/// Kelvin sign.
pub(super) fn is_protocol_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-' | '\u{130}' | '\u{131}' | '\u{17f}' | '\u{212a}')
}

/// A character a hostname label may have: `[+a-z0-9A-Z_-]`.
fn is_hostname_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '_' | '-')
}

/// mdurl's `url_parse(url, slashes_denote_host=True)`, as markdown-it calls it.
fn parse(url: &str) -> Url {
    let mut rest = url.trim_matches(is_space).to_owned();

    let scheme_len = rest.find(|c: char| !is_protocol_char(c)).unwrap_or(rest.len());
    let protocol = (scheme_len > 0 && rest[scheme_len..].starts_with(':')).then(|| rest[..=scheme_len].to_owned());
    if let Some(protocol) = &protocol {
        rest.drain(..protocol.len());
    }
    let proto = protocol.as_deref().unwrap_or_default();
    // Case-sensitive lookups, as mdurl's tables are.
    let hostless = matches!(proto, "javascript" | "javascript:");
    let always_slashed = matches!(proto, "http:" | "https:" | "ftp:" | "gopher:" | "file:");

    let starts_with_slashes = rest.starts_with("//");
    let slashes = starts_with_slashes && !(protocol.is_some() && hostless);
    if slashes {
        rest.drain(..2);
    }

    let has_host = !hostless && (starts_with_slashes || (protocol.is_some() && !always_slashed));
    let (auth, port, hostname) = if has_host {
        take_host(&mut rest)
    } else {
        Default::default()
    };

    let hash = rest.find('#').map(|at| rest.split_off(at)).unwrap_or_default();
    let search = rest.find('?').map(|at| rest.split_off(at)).unwrap_or_default();
    Url {
        protocol,
        slashes,
        auth,
        port,
        hostname,
        hash,
        search,
        pathname: rest,
    }
}

/// The auth, port and hostname at the start of `rest`, taken off it.
fn take_host(rest: &mut String) -> (String, String, String) {
    // The auth ends at the last `@` before the first `/`, `?` or `#`.
    let host_end = rest.find(['/', '?', '#']);
    let at_sign = match host_end {
        Some(end) => rest[..end].rfind('@'),
        None => rest.rfind('@'),
    };
    let mut auth = String::new();
    if let Some(at) = at_sign {
        auth = rest.drain(..=at).collect();
        auth.pop();
    }

    let mut host_end = rest.find(NON_HOST_CHARS).unwrap_or(rest.len());
    if rest[..host_end].ends_with(':') {
        host_end -= 1;
    }
    let host: String = rest.drain(..host_end).collect();
    let (name, port) = split_port(&host);
    let mut hostname = name.to_owned();

    let ipv6 = hostname.starts_with('[') && hostname.ends_with(']');
    if !ipv6 {
        validate_hostname(&mut hostname, rest);
    }
    if hostname.chars().count() > HOSTNAME_MAX_LEN {
        hostname.clear();
    }
    if ipv6 {
        // `[` alone both starts and ends with a bracket, and becomes empty.
        hostname = hostname.get(1..hostname.len() - 1).unwrap_or_default().to_owned();
    }
    (auth, port.to_owned(), hostname)
}

/// mdurl's `parse_host`: a trailing `:` and digits is the port. The hostname and the port.
fn split_port(host: &str) -> (&str, &str) {
    match host.rfind(':') {
        Some(colon) if host[colon + 1..].bytes().all(|b| b.is_ascii_digit()) => (&host[..colon], &host[colon + 1..]),
        _ => (host, ""),
    }
}

/// mdurl's label check: at the first label (between dots) that isn't up to 63 of `[+a-z0-9A-Z_-]` (a non-ASCII
/// character counting as one), the hostname ends after that label's valid start, and the rest goes back to `rest`.
fn validate_hostname(hostname: &mut String, rest: &mut String) {
    let parts: Vec<&str> = hostname.split('.').collect();
    let valid = |part: &str| part.chars().count() <= 63 && part.chars().all(|c| !c.is_ascii() || is_hostname_char(c));
    let Some(bad) = parts.iter().position(|part| !part.is_empty() && !valid(part)) else {
        return;
    };
    let part = parts[bad];
    let good_len: usize = part
        .chars()
        .take(63)
        .take_while(|&c| is_hostname_char(c))
        .map(char::len_utf8)
        .sum();
    let mut valid_parts = parts[..bad].to_vec();
    valid_parts.push(&part[..good_len]);
    let mut not_host = vec![&part[good_len..]];
    not_host.extend_from_slice(&parts[bad + 1..]);
    let new_rest = not_host.join(".") + rest.as_str();
    let new_hostname = valid_parts.join(".");
    *rest = new_rest;
    *hostname = new_hostname;
}

/// mdurl's `encode` with its default exclusions: percent-encodes everything but ASCII letters, digits,
/// `;/?:@&=+$,-_.!~*'()#` and `%` starting a valid escape. Non-ASCII characters are encoded as UTF-8.
fn encode(text: &str) -> String {
    const KEEP: &str = ";/?:@&=+$,-_.!~*'()#";
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    for (at, c) in text.char_indices() {
        let escape = c == '%'
            && bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit);
        if escape || c.is_ascii_alphanumeric() || KEEP.contains(c) {
            out.push(c);
        } else {
            let mut buf = [0; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                push_escape(&mut out, byte);
            }
        }
    }
    out
}

/// Appends `%XX`, upper-case, for `byte`.
fn push_escape(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(char::from(HEX[usize::from(byte >> 4)]));
    out.push(char::from(HEX[usize::from(byte & 0xF)]));
}

/// mdurl's `decode`: each run of `%XX` escapes decoded as UTF-8, a bad sequence as U+FFFD per byte, except escapes of
/// the ASCII characters in `exclude`, which stay (upper-cased).
fn decode(text: &str, exclude: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let mut run = Vec::new();
        while let Some(byte) = escaped_byte(rest) {
            run.push(byte);
            rest = &rest[3..];
        }
        if run.is_empty() {
            out.push('%');
            rest = &rest[1..];
        } else {
            decode_run(&run, exclude, &mut out);
        }
    }
    out.push_str(rest);
    out
}

/// The byte a `%XX` at the start of `text` stands for.
fn escaped_byte(text: &str) -> Option<u8> {
    let hex = text.strip_prefix('%')?.get(..2)?;
    hex.bytes()
        .all(|b| b.is_ascii_hexdigit())
        .then(|| u8::from_str_radix(hex, 16).ok())
        .flatten()
}

/// One run of escaped bytes, as mdurl's `repl_func_with_cache` decodes it.
fn decode_run(bytes: &[u8], exclude: &str, out: &mut String) {
    let mut at = 0;
    while let Some(&lead) = bytes.get(at) {
        if lead < 0x80 {
            let c = char::from(lead);
            if exclude.contains(c) {
                push_escape(out, lead);
            } else {
                out.push(c);
            }
            at += 1;
            continue;
        }
        let width = match lead {
            _ if lead & 0xE0 == 0xC0 => 2,
            _ if lead & 0xF0 == 0xE0 => 3,
            _ if lead & 0xF8 == 0xF0 => 4,
            _ => 0,
        };
        let sequence = bytes
            .get(at..at + width)
            .filter(|seq| width > 0 && seq[1..].iter().all(|b| b & 0xC0 == 0x80));
        if let Some(sequence) = sequence {
            match std::str::from_utf8(sequence) {
                Ok(text) => out.push_str(text),
                // Overlong forms, surrogates and code points past U+10FFFF: one replacement per byte.
                Err(_) => out.extend(std::iter::repeat_n('\u{fffd}', width)),
            }
            at += width;
        } else {
            out.push('\u{fffd}');
            at += 1;
        }
    }
}

/// markdown-it's `_punycode.map_domain`: `f` on each label of the domain (split on `.`, `。`, `．` and `｡`, joined
/// with `.`), after an email's `user@`. Anything after a second `@` is dropped, as in markdown-it.
fn map_domain(domain: &str, f: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut parts = domain.split('@');
    let first = parts.next().unwrap_or_default();
    let (prefix, domain) = match parts.next() {
        Some(domain) => (format!("{first}@"), domain),
        None => (String::new(), first),
    };
    let labels: Option<Vec<String>> = domain.split(['.', '\u{3002}', '\u{ff0e}', '\u{ff61}']).map(f).collect();
    Some(prefix + &labels?.join("."))
}

/// markdown-it's `to_ascii`: each label with a character past `~` as `xn--` and its punycode.
fn to_ascii(domain: &str) -> String {
    let encoded = map_domain(domain, |label| {
        Some(if label.chars().any(|c| c > '\u{7e}') {
            format!("xn--{}", punycode::encode(label))
        } else {
            label.to_owned()
        })
    });
    // Encoding can't fail, so `map_domain` always has a result.
    encoded.unwrap_or_else(|| domain.to_owned())
}

/// markdown-it's `to_unicode`: each `xn--` label decoded from its (lower-cased) punycode. `None` if one doesn't
/// decode, which leaves the hostname as it was.
fn to_unicode(domain: &str) -> Option<String> {
    map_domain(domain, |label| match label.strip_prefix("xn--") {
        Some(code) => punycode::decode(&code.to_lowercase()),
        None => Some(label.to_owned()),
    })
}

/// RFC 3492 punycode as Python's `punycode` codec does it: basic code points keep their case, and the decoder takes
/// digits in either case.
mod punycode {
    const BASE: u64 = 36;
    const T_MIN: u64 = 1;
    const T_MAX: u64 = 26;
    const SKEW: u64 = 38;
    const DAMP: u64 = 700;
    const INITIAL_BIAS: u64 = 72;
    const INITIAL_N: u64 = 0x80;

    /// The threshold for the `j`th digit of a number.
    fn threshold(j: u64, bias: u64) -> u64 {
        (BASE * (j + 1)).saturating_sub(bias).clamp(T_MIN, T_MAX)
    }

    fn adapt(delta: u64, first: bool, points: u64) -> u64 {
        let mut delta = if first { delta / DAMP } else { delta / 2 };
        delta += delta / points;
        let mut k = 0;
        while delta > ((BASE - T_MIN) * T_MAX) / 2 {
            delta /= BASE - T_MIN;
            k += BASE;
        }
        k + (BASE * delta) / (delta + SKEW)
    }

    fn digit(d: u64) -> char {
        // `d` is under 36 by construction.
        char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[usize::try_from(d).unwrap_or_default()])
    }

    /// The punycode of `input`, without `xn--`.
    pub(super) fn encode(input: &str) -> String {
        let points: Vec<u64> = input.chars().map(u64::from).collect();
        let mut out: String = input.chars().filter(char::is_ascii).collect();
        let basic = out.len() as u64;
        if basic > 0 {
            out.push('-');
        }
        let (mut n, mut delta, mut bias, mut handled) = (INITIAL_N, 0_u64, INITIAL_BIAS, basic);
        let total = points.len() as u64;
        while handled < total {
            // Some point at or past `n` is left while `handled < total`.
            let Some(&m) = points.iter().filter(|&&p| p >= n).min() else {
                break;
            };
            delta += (m - n) * (handled + 1);
            n = m;
            for &p in &points {
                if p < n {
                    delta += 1;
                }
                if p == n {
                    let mut q = delta;
                    for j in 0.. {
                        let t = threshold(j, bias);
                        if q < t {
                            break;
                        }
                        out.push(digit(t + (q - t) % (BASE - t)));
                        q = (q - t) / (BASE - t);
                    }
                    out.push(digit(q));
                    bias = adapt(delta, handled == basic, handled + 1);
                    delta = 0;
                    handled += 1;
                }
            }
            delta += 1;
            n += 1;
        }
        out
    }

    /// Decodes punycode without `xn--`: `None` if it isn't ASCII, is cut short, has a character that isn't a digit
    /// after the last `-`, or decodes to a code point that isn't a character.
    pub(super) fn decode(input: &str) -> Option<String> {
        if !input.is_ascii() {
            return None;
        }
        let (base, extended) = match input.rfind('-') {
            Some(dash) => (&input[..dash], &input[dash + 1..]),
            None => ("", input),
        };
        let mut out: Vec<char> = base.chars().collect();
        let extended = extended.as_bytes();
        let (mut n, mut i, mut bias, mut at) = (INITIAL_N, 0_u64, INITIAL_BIAS, 0);
        let mut first = true;
        while at < extended.len() {
            let (next, delta) = number(extended, at, bias)?;
            // Python's `pos` starts at -1 and adds `delta + 1`.
            i = i.checked_add(delta)?;
            let len = out.len() as u64 + 1;
            n = n.checked_add(i / len)?;
            if n > 0x10_FFFF {
                return None;
            }
            i %= len;
            // A surrogate, which Python would keep in its string.
            let c = char::from_u32(u32::try_from(n).ok()?)?;
            out.insert(usize::try_from(i).ok()?, c);
            bias = adapt(delta, first, out.len() as u64);
            first = false;
            at = next;
            i += 1;
        }
        Some(out.into_iter().collect())
    }

    /// One generalized variable-length integer from `at`: where it ends, and its value.
    fn number(digits: &[u8], mut at: usize, bias: u64) -> Option<(usize, u64)> {
        let mut result = 0_u64;
        // `None` once the weight is past `u64`: only zero digits can follow without the result overflowing too.
        let mut weight = Some(1_u64);
        let mut j = 0;
        loop {
            let byte = *digits.get(at)?;
            at += 1;
            let d = match byte {
                b'a'..=b'z' => byte - b'a',
                b'A'..=b'Z' => byte - b'A',
                b'0'..=b'9' => byte - b'0' + 26,
                _ => return None,
            };
            let d = u64::from(d);
            if d > 0 {
                result = result.checked_add(d.checked_mul(weight?)?)?;
            }
            let t = threshold(j, bias);
            if d < t {
                return Some((at, result));
            }
            weight = weight.and_then(|w| w.checked_mul(BASE - t));
            j += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case("http://a.com/[x]", "http://a.com/%5Bx%5D")]
    #[case("http://a.com/{x}|y", "http://a.com/%7Bx%7D%7Cy")]
    #[case("http://a.com/`x`", "http://a.com/%60x%60")]
    #[case("http://a.com/~x^y", "http://a.com/~x%5Ey")]
    #[case("http://a.com/a b", "http://a.com/a%20b")]
    #[case("http://a.com/ä", "http://a.com/%C3%A4")]
    #[case("http://a.com/%zz", "http://a.com/%25zz")]
    #[case("http://a.com/%C3%A4", "http://a.com/%C3%A4")]
    #[case("http://a.com/%c3", "http://a.com/%c3")]
    #[case("http://a.com/%", "http://a.com/%25")]
    #[case("http://a.com/%4", "http://a.com/%254")]
    #[case("http://a.com/😀", "http://a.com/%F0%9F%98%80")]
    #[case("http://ä.com/p", "http://xn--4ca.com/p")]
    #[case("http://Bücher.example/", "http://xn--Bcher-kva.example/")]
    #[case(
        "https://user:pw@ä.com:8080/x?q=ä#ä",
        "https://user:pw@xn--4ca.com:8080/x?q=%C3%A4#%C3%A4"
    )]
    #[case("http://ä。com/", "http://xn--4ca.com/")]
    #[case("mailto:foo@bär.com", "mailto:foo@xn--br-via.com")]
    #[case("//ä.com/x", "//xn--4ca.com/x")]
    #[case("ä.com/x", "%C3%A4.com/x")]
    #[case("HTTP://ä.com", "HTTP://%C3%A4.com")]
    #[case("skype:ä", "skype:%C3%A4")]
    #[case("ftp://ä.com/", "ftp://%C3%A4.com/")]
    #[case("http://[::1]:80/x", "http://%5B::1%5D:80/x")]
    #[case("http://a.com:/x", "http://a.com:/x")]
    #[case("  http://a.com  ", "http://a.com")]
    #[case("http://@a.com", "http://a.com")]
    #[case("http://a!b.com/x", "http://a!b.com/x")]
    #[case("javascript:alert(1)", "javascript:alert(1)")]
    #[case("", "")]
    fn normalize_link_matches_markdown_it(#[case] url: &str, #[case] expected: &str) {
        assert_eq!(normalize_link(url), expected);
    }

    #[rstest::rstest]
    #[case("http://a.com/%20x", "http://a.com/ x")]
    #[case("http://a.com/%C3%A4", "http://a.com/ä")]
    #[case("http://a.com/a%2Fb", "http://a.com/a%2Fb")]
    #[case("http://a.com/a%2fb", "http://a.com/a%2Fb")]
    #[case("http://a.com/%25", "http://a.com/%25")]
    #[case("http://a.com/%C3", "http://a.com/\u{fffd}")]
    #[case("http://a.com/%C0%80", "http://a.com/\u{fffd}\u{fffd}")]
    #[case("http://a.com/%ED%A0%80", "http://a.com/\u{fffd}\u{fffd}\u{fffd}")]
    #[case("http://a.com/%F0%9F%98%80", "http://a.com/😀")]
    #[case("http://a.com/%zz", "http://a.com/%zz")]
    #[case("http://xn--4ca.com/p", "http://ä.com/p")]
    #[case("http://XN--4CA.com/p", "http://XN--4CA.com/p")]
    #[case("http://xn--4CA.com/p", "http://ä.com/p")]
    #[case("http://xn--Bcher-kva.example/", "http://bücher.example/")]
    #[case("http://xn--zz!.com/p", "http://xn--zz!.com/p")]
    #[case("http://xn--.com/", "http://.com/")]
    #[case("www.xn--bcher-kva.de", "www.xn--bcher-kva.de")]
    #[case("http://www.xn--bcher-kva.de", "http://www.bücher.de")]
    #[case("mailto:x@xn--br-via.com", "mailto:x@bär.com")]
    #[case("//xn--4ca.com/x", "//ä.com/x")]
    #[case("skype:xn--4ca", "skype:xn--4ca")]
    fn normalize_link_text_matches_markdown_it(#[case] url: &str, #[case] expected: &str) {
        assert_eq!(normalize_link_text(url), expected);
    }

    #[rstest::rstest]
    #[case("http://a.com", true)]
    #[case("javascript:alert(1)", false)]
    #[case(" JavaScript:x", false)]
    #[case("vbscript:x", false)]
    #[case("file:///etc/passwd", false)]
    #[case("data:text/html;x", false)]
    #[case("data:image/png;base64,x", true)]
    #[case("DATA:IMAGE/WEBP;x", true)]
    #[case("data:image/svg+xml;x", false)]
    #[case("javascript", true)]
    fn validate_link_matches_markdown_it(#[case] url: &str, #[case] valid: bool) {
        assert_eq!(validate_link(url), valid);
    }

    #[test]
    fn parse_splits_like_mdurl() {
        let url = parse("https://u:p@a.com:8080/p/q?x=1#f");
        assert_eq!(
            url,
            Url {
                protocol: Some("https:".into()),
                slashes: true,
                auth: "u:p".into(),
                port: "8080".into(),
                hostname: "a.com".into(),
                hash: "#f".into(),
                search: "?x=1".into(),
                pathname: "/p/q".into(),
            }
        );
        // A bad label ends the host, and what's left of it starts the path.
        let url = parse("http://a.b*c.d/e");
        assert_eq!((url.hostname.as_str(), url.pathname.as_str()), ("a.b", "*c.d/e"));
        // An unknown scheme has a host even without `//`.
        assert_eq!(parse("skype:me@host").hostname, "host");
        assert_eq!(parse("http:x").hostname, "");
        assert_eq!(parse("javascript://a.com").hostname, "");
        assert_eq!(parse(&format!("http://{}a/", "a.".repeat(127))).hostname.len(), 255);
        assert_eq!(parse(&format!("http://{}a/", "a.".repeat(128))).hostname, "");
        assert_eq!(parse("http://[/x").hostname, "");
    }

    /// RFC 3492 section 7.1's samples, as Python's `punycode` codec encodes and decodes them.
    #[rstest::rstest]
    #[case(
        "\u{644}\u{64a}\u{647}\u{645}\u{627}\u{628}\u{62a}\u{643}\u{644}\u{645}\u{648}\u{634}\u{639}\u{631}\u{628}\u{64a}\u{61f}",
        "egbpdaj6bu4bxfgehfvwxn"
    )]
    #[case(
        "\u{4ed6}\u{4eec}\u{4e3a}\u{4ec0}\u{4e48}\u{4e0d}\u{8bf4}\u{4e2d}\u{6587}",
        "ihqwcrb4cv8a8dqg056pqjye"
    )]
    #[case("Pročprostěnemluvíčesky", "Proprostnemluvesky-uyb24dma41a")]
    #[case(
        "PorquénopuedensimplementehablarenEspañol",
        "PorqunopuedensimplementehablarenEspaol-fmd56a"
    )]
    #[case("3年B組金八先生", "3B-ww4c5e180e575a65lsy2b")]
    #[case("安室奈美恵-with-SUPER-MONKEYS", "-with-SUPER-MONKEYS-pc58ag80a8qai00g7n9n")]
    #[case("MajiでKoiする5秒前", "MajiKoi5-783gue6qz075azm5e")]
    #[case("Bücher", "Bcher-kva")]
    #[case("ä", "4ca")]
    fn punycode_round_trips(#[case] unicode: &str, #[case] ascii: &str) {
        assert_eq!(punycode::encode(unicode), ascii);
        assert_eq!(punycode::decode(ascii).as_deref(), Some(unicode));
    }

    #[test]
    fn punycode_rejects_what_python_does() {
        for bad in [
            "ä",
            "a-!",
            "a-9",
            "zzzzzzzzzzzzzzzzzzzzzzzz",
            "99999999999999999999999999",
        ] {
            assert_eq!(punycode::decode(bad), None, "{bad}");
        }
        // The decoder reads digits in either case.
        assert_eq!(punycode::decode("Bcher-KVA").as_deref(), Some("Bücher"));
    }

    #[test]
    fn email_domains_keep_the_user() {
        assert_eq!(to_ascii("ü@bär.com"), "ü@xn--br-via.com");
        assert_eq!(to_ascii("a@b@ä.com"), "a@b");
        assert_eq!(to_unicode("x@xn--br-via.com").as_deref(), Some("x@bär.com"));
    }
}
