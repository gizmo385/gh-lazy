//! Parsing the subset of gh's command line that the apps send.
//!
//! gh uses cobra, which lets flags appear before or after the subcommand (`gh --method PATCH api /x` works), so
//! flags are collected from anywhere and checked against the subcommand afterwards.

use std::io;

use serde_json::{Map, Value};

use crate::fixture::Accept;

/// A parsed invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Api(ApiRequest),
    AuthStatus,
    AuthToken,
    Version,
    PrDiff {
        owner: String,
        repo: String,
        number: u64,
        patch: bool,
    },
}

impl Command {
    /// The `command` value written to the request log.
    pub fn log_name(&self) -> &'static str {
        match self {
            Command::Api(_) => "api",
            Command::AuthStatus => "auth status",
            Command::AuthToken => "auth token",
            Command::Version => "version",
            Command::PrDiff { .. } => "pr diff",
        }
    }
}

/// `gh api` with its flags, before `-F @file` and `--input` are read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ApiRequest {
    pub endpoint: String,
    /// `-X/--method` as given (uppercased); see [`ApiRequest::method`] for the effective method.
    pub method: Option<String>,
    /// `-H/--header` values split at the first `:`, trimmed, in order.
    pub headers: Vec<(String, String)>,
    /// `-F`/`-f` fields in order.
    pub fields: Vec<Field>,
    /// `--input` path, or `-` for stdin.
    pub input: Option<String>,
    pub include: bool,
    pub cache: Option<String>,
    pub paginate: bool,
    pub slurp: bool,
}

/// One `-F k=v` (typed) or `-f k=v` (raw) field.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub key: String,
    pub value: String,
    /// `-f/--raw-field`: always a string, and `@` is not special.
    pub raw: bool,
}

impl ApiRequest {
    pub fn is_graphql(&self) -> bool {
        self.endpoint == "graphql"
    }

    /// gh's rule: an explicit method wins; otherwise POST when fields or `--input` are given (or for `graphql`),
    /// else GET.
    pub fn method(&self) -> String {
        match &self.method {
            Some(m) => m.clone(),
            // A bare `gh api graphql` is a GET in real gh; POST here is #67's choice, and every real GraphQL call
            // carries `-f query=` anyway.
            None if !self.fields.is_empty() || self.input.is_some() || self.is_graphql() => "POST".into(),
            None => "GET".into(),
        }
    }

    /// Where the fields go. gh sends them as query params for GET, and also when `--input` supplies the body.
    pub fn fields_in_query(&self) -> bool {
        self.method() == "GET" || self.input.is_some()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The normalized `Accept`; the last header wins.
    pub fn accept(&self) -> Accept {
        Accept::normalize(self.header("accept").unwrap_or(""))
    }

    pub fn cache_control(&self) -> Option<&str> {
        self.header("cache-control")
    }
}

/// Converts fields to typed values the way gh's `-F` does: `true`, `false`, `null` and integers become JSON
/// literals and `@path` reads a file (`@-` reads stdin). `-f` values stay strings. `read` loads `@` files.
pub fn typed_fields(
    fields: &[Field],
    read: &mut dyn FnMut(&str) -> io::Result<String>,
) -> io::Result<Vec<(String, Value)>> {
    fields
        .iter()
        .map(|f| {
            let value = if f.raw {
                Value::String(f.value.clone())
            } else if let Some(path) = f.value.strip_prefix('@') {
                Value::String(read(path)?)
            } else {
                match f.value.as_str() {
                    "true" => Value::Bool(true),
                    "false" => Value::Bool(false),
                    "null" => Value::Null,
                    v => v
                        .parse::<i64>()
                        .map_or_else(|_| Value::String(v.to_owned()), Value::from),
                }
            };
            Ok((f.key.clone(), value))
        })
        .collect()
}

/// Typed fields as query pairs, formatted the way gh puts them on the URL.
pub fn query_pairs(fields: &[(String, Value)]) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(k, v)| {
            let s = match v {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (k.clone(), s)
        })
        .collect()
}

/// Typed fields as a JSON object body. `k[]=v` appends to an array at `k`. For `graphql`, everything but `query`
/// and `operationName` goes under `variables`, as gh does.
pub fn json_body(fields: &[(String, Value)], graphql: bool) -> Value {
    let mut top = Map::new();
    let mut variables = Map::new();
    for (key, value) in fields {
        let target = if graphql && key != "query" && key != "operationName" {
            &mut variables
        } else {
            &mut top
        };
        if let Some(name) = key.strip_suffix("[]") {
            let slot = target
                .entry(name.to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(items) = slot {
                items.push(value.clone());
            } else {
                *slot = Value::Array(vec![value.clone()]);
            }
        } else {
            target.insert(key.clone(), value.clone());
        }
    }
    if !variables.is_empty() {
        top.insert("variables".into(), Value::Object(variables));
    }
    Value::Object(top)
}

/// Why argv was refused. fake-gh prints `fake-gh: <error>` and exits 2.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgvError {
    #[error("unsupported flag {0}")]
    UnsupportedFlag(String),
    #[error("flag needs an argument: {0}")]
    MissingValue(String),
    #[error("unsupported command: gh {0}")]
    UnsupportedCommand(String),
    #[error("unsupported host {0}: fake-gh only serves github.com")]
    UnsupportedHost(String),
    #[error("invalid value {value:?} for {flag}")]
    InvalidValue { flag: String, value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flag {
    Method,
    Header,
    Field,
    RawField,
    Input,
    Include,
    Cache,
    Paginate,
    Slurp,
    Hostname,
    Repo,
    Color,
    Patch,
    Active,
    ShowToken,
    Version,
}

/// (short, long, flag, takes a value)
const FLAGS: &[(Option<char>, &str, Flag, bool)] = &[
    (Some('X'), "method", Flag::Method, true),
    (Some('H'), "header", Flag::Header, true),
    (Some('F'), "field", Flag::Field, true),
    (Some('f'), "raw-field", Flag::RawField, true),
    (None, "input", Flag::Input, true),
    (Some('i'), "include", Flag::Include, false),
    (None, "cache", Flag::Cache, true),
    (None, "paginate", Flag::Paginate, false),
    (None, "slurp", Flag::Slurp, false),
    (None, "hostname", Flag::Hostname, true),
    (Some('R'), "repo", Flag::Repo, true),
    (None, "color", Flag::Color, true),
    (None, "patch", Flag::Patch, false),
    (Some('a'), "active", Flag::Active, false),
    (Some('t'), "show-token", Flag::ShowToken, false),
    (None, "version", Flag::Version, false),
];

struct ParsedFlag {
    flag: Flag,
    /// How it was spelled, for error messages.
    spelling: String,
    value: Option<String>,
}

/// Parses gh's arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Command, ArgvError> {
    let (flags, positionals) = split(args)?;
    let pos: Vec<&str> = positionals.iter().map(String::as_str).collect();
    let unsupported = || ArgvError::UnsupportedCommand(args.join(" "));
    let allow = |allowed: &[Flag]| -> Result<(), ArgvError> {
        match flags.iter().find(|f| !allowed.contains(&f.flag)) {
            Some(f) => Err(ArgvError::UnsupportedFlag(f.spelling.clone())),
            None => Ok(()),
        }
    };
    for f in flags.iter().filter(|f| f.flag == Flag::Hostname) {
        let host = f.value.clone().unwrap_or_default();
        if host != "github.com" {
            return Err(ArgvError::UnsupportedHost(host));
        }
    }

    match pos.as_slice() {
        [] if flags.iter().any(|f| f.flag == Flag::Version) => {
            allow(&[Flag::Version])?;
            Ok(Command::Version)
        }
        ["version"] => {
            allow(&[])?;
            Ok(Command::Version)
        }
        ["auth", "status"] => {
            allow(&[Flag::Hostname, Flag::Active, Flag::ShowToken])?;
            Ok(Command::AuthStatus)
        }
        ["auth", "token"] => {
            allow(&[Flag::Hostname])?;
            Ok(Command::AuthToken)
        }
        ["pr", "diff", number] => {
            allow(&[Flag::Repo, Flag::Color, Flag::Patch])?;
            parse_pr_diff(number, &flags).ok_or_else(unsupported)?
        }
        ["api", endpoint] => {
            allow(&[
                Flag::Method,
                Flag::Header,
                Flag::Field,
                Flag::RawField,
                Flag::Input,
                Flag::Include,
                Flag::Cache,
                Flag::Paginate,
                Flag::Slurp,
                Flag::Hostname,
            ])?;
            parse_api(endpoint, flags).map(Command::Api)
        }
        _ => Err(unsupported()),
    }
}

/// `None` means the shape isn't supported (a non-numeric selector, or no `-R`, which gh would resolve from git).
fn parse_pr_diff(number: &str, flags: &[ParsedFlag]) -> Option<Result<Command, ArgvError>> {
    let number = number.parse().ok()?;
    let repo = flags.iter().rev().find(|f| f.flag == Flag::Repo)?.value.clone()?;
    let Some((owner, name)) = repo
        .split_once('/')
        .filter(|(o, r)| !o.is_empty() && !r.is_empty() && !r.contains('/'))
    else {
        return Some(Err(ArgvError::InvalidValue {
            flag: "--repo".into(),
            value: repo.clone(),
        }));
    };
    for f in flags.iter().filter(|f| f.flag == Flag::Color) {
        let v = f.value.as_deref().unwrap_or_default();
        if !matches!(v, "never" | "auto" | "always") {
            return Some(Err(ArgvError::InvalidValue {
                flag: "--color".into(),
                value: v.into(),
            }));
        }
    }
    Some(Ok(Command::PrDiff {
        owner: owner.to_owned(),
        repo: name.to_owned(),
        number,
        patch: flags.iter().any(|f| f.flag == Flag::Patch),
    }))
}

fn parse_api(endpoint: &str, flags: Vec<ParsedFlag>) -> Result<ApiRequest, ArgvError> {
    let mut req = ApiRequest {
        endpoint: endpoint.to_owned(),
        ..ApiRequest::default()
    };
    for f in flags {
        let value = f.value.unwrap_or_default();
        let invalid = || ArgvError::InvalidValue {
            flag: f.spelling.clone(),
            value: value.clone(),
        };
        match f.flag {
            Flag::Method => req.method = Some(value.to_ascii_uppercase()),
            Flag::Header => {
                let (name, v) = value.split_once(':').ok_or_else(invalid)?;
                req.headers.push((name.trim().to_owned(), v.trim().to_owned()));
            }
            Flag::Field | Flag::RawField => {
                let (k, v) = value.split_once('=').ok_or_else(invalid)?;
                req.fields.push(Field {
                    key: k.to_owned(),
                    value: v.to_owned(),
                    raw: f.flag == Flag::RawField,
                });
            }
            Flag::Input => req.input = Some(value),
            Flag::Include => req.include = true,
            Flag::Cache => req.cache = Some(value),
            Flag::Paginate => req.paginate = true,
            Flag::Slurp => req.slurp = true,
            _ => {}
        }
    }
    Ok(req)
}

fn lookup_long(name: &str) -> Option<(Flag, bool)> {
    FLAGS
        .iter()
        .find(|(_, long, _, _)| *long == name)
        .map(|&(_, _, f, v)| (f, v))
}

fn lookup_short(c: char) -> Option<(Flag, bool)> {
    FLAGS
        .iter()
        .find(|(short, _, _, _)| *short == Some(c))
        .map(|&(_, _, f, v)| (f, v))
}

fn split(args: &[String]) -> Result<(Vec<ParsedFlag>, Vec<String>), ArgvError> {
    let mut flags = Vec::new();
    let mut positionals = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            positionals.extend(iter.cloned());
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_owned())),
                None => (long, None),
            };
            let spelling = format!("--{name}");
            let (flag, takes_value) = lookup_long(name).ok_or_else(|| ArgvError::UnsupportedFlag(spelling.clone()))?;
            let value = if takes_value {
                Some(match inline {
                    Some(v) => v,
                    None => iter
                        .next()
                        .cloned()
                        .ok_or_else(|| ArgvError::MissingValue(spelling.clone()))?,
                })
            } else if inline.is_some() {
                return Err(ArgvError::InvalidValue {
                    flag: spelling,
                    value: inline.unwrap_or_default(),
                });
            } else {
                None
            };
            flags.push(ParsedFlag { flag, spelling, value });
        } else if arg.len() > 1 && arg.starts_with('-') {
            // Short flags; cobra allows `-iX GET` and `-XGET`.
            for (i, c) in arg[1..].char_indices() {
                let spelling = format!("-{c}");
                let (flag, takes_value) =
                    lookup_short(c).ok_or_else(|| ArgvError::UnsupportedFlag(spelling.clone()))?;
                if takes_value {
                    let rest = &arg[1 + i + c.len_utf8()..];
                    let rest = rest.strip_prefix('=').unwrap_or(rest);
                    let value = if rest.is_empty() {
                        iter.next()
                            .cloned()
                            .ok_or_else(|| ArgvError::MissingValue(spelling.clone()))?
                    } else {
                        rest.to_owned()
                    };
                    flags.push(ParsedFlag {
                        flag,
                        spelling,
                        value: Some(value),
                    });
                    break;
                }
                flags.push(ParsedFlag {
                    flag,
                    spelling,
                    value: None,
                });
            }
        } else {
            positionals.push(arg.clone());
        }
    }
    Ok((flags, positionals))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| (*a).to_owned()).collect()
    }

    fn api(s: &[&str]) -> ApiRequest {
        match parse(&args(s)) {
            Ok(Command::Api(req)) => req,
            other => panic!("expected api, got {other:?}"),
        }
    }

    fn no_files(_: &str) -> io::Result<String> {
        Err(io::Error::other("unexpected read"))
    }

    #[test]
    fn python_rest_get() {
        // build_command('/user/repos', headers=github_headers(...), query_params={...})
        let req = api(&[
            "api",
            "-i",
            "-X",
            "GET",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "Cache-Control: max-age=600",
            "-F",
            "type=all",
            "-F",
            "direction=asc",
            "-F",
            "sort=full_name",
            "-F",
            "page=1",
            "-F",
            "per_page=50",
            "/user/repos",
        ]);
        assert!(req.include);
        assert_eq!(req.method(), "GET");
        assert!(req.fields_in_query());
        assert_eq!(req.accept(), Accept::Json);
        assert_eq!(req.cache_control(), Some("max-age=600"));
        assert_eq!(req.endpoint, "/user/repos");
        let typed = typed_fields(&req.fields, &mut no_files).unwrap();
        assert_eq!(
            query_pairs(&typed),
            [
                ("type", "all"),
                ("direction", "asc"),
                ("sort", "full_name"),
                ("page", "1"),
                ("per_page", "50")
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
    }

    #[test]
    fn python_diff_accept() {
        let req = api(&[
            "api",
            "-i",
            "-X",
            "GET",
            "-H",
            "Accept: application/vnd.github.diff",
            "/repos/o/r/pulls/1",
        ]);
        assert_eq!(req.accept(), Accept::Diff);
    }

    #[test]
    fn python_post_with_input_file() {
        let req = api(&[
            "api",
            "-i",
            "-X",
            "POST",
            "-H",
            "Accept: application/vnd.github+json",
            "--input",
            "/home/u/.config/lazy-github/request_bodies/tmpab12",
            "/repos/o/r/issues",
        ]);
        assert_eq!(req.method(), "POST");
        assert_eq!(
            req.input.as_deref(),
            Some("/home/u/.config/lazy-github/request_bodies/tmpab12")
        );
    }

    #[test]
    fn python_patch_put_and_bodyless_post() {
        assert_eq!(
            api(&["api", "-i", "-X", "PATCH", "--input", "f", "/repos/o/r/issues/1"]).method(),
            "PATCH"
        );
        assert_eq!(
            api(&["api", "-i", "-X", "PUT", "--input", "f", "/repos/o/r/pulls/1/merge"]).method(),
            "PUT"
        );
        let rerun = api(&[
            "api",
            "-i",
            "-X",
            "POST",
            "-H",
            "Accept: application/vnd.github+json",
            "/x/rerun",
        ]);
        assert_eq!(rerun.method(), "POST");
        assert_eq!(rerun.input, None);
    }

    #[test]
    fn python_notifications_list() {
        let req = api(&[
            "api",
            "-i",
            "-X",
            "GET",
            "-F",
            "all=false",
            "-F",
            "page=1",
            "-F",
            "per_page=50",
            "/notifications",
        ]);
        assert!(req.headers.is_empty());
        let typed = typed_fields(&req.fields, &mut no_files).unwrap();
        assert_eq!(typed[0], ("all".to_owned(), Value::Bool(false)));
        assert_eq!(query_pairs(&typed)[0], ("all".to_owned(), "false".to_owned()));
    }

    #[test]
    fn method_flag_before_subcommand() {
        // notifications.py: gh --method PATCH api /notifications/threads/<id>
        let req = api(&["--method", "PATCH", "api", "/notifications/threads/123"]);
        assert_eq!(req.method(), "PATCH");
        assert!(!req.include);
        assert_eq!(req.endpoint, "/notifications/threads/123");
    }

    #[test]
    fn python_absolute_url() {
        let req = api(&[
            "api",
            "-i",
            "-X",
            "GET",
            "https://api.github.com/repos/o/r/issues/1/comments",
        ]);
        assert_eq!(req.endpoint, "https://api.github.com/repos/o/r/issues/1/comments");
    }

    #[test]
    fn auth_version_and_pr_diff() {
        assert_eq!(parse(&args(&["auth", "status"])), Ok(Command::AuthStatus));
        assert_eq!(
            parse(&args(&["auth", "status", "--hostname", "github.com", "--active", "-t"])),
            Ok(Command::AuthStatus)
        );
        assert_eq!(parse(&args(&["auth", "token"])), Ok(Command::AuthToken));
        assert_eq!(parse(&args(&["--version"])), Ok(Command::Version));
        assert_eq!(parse(&args(&["version"])), Ok(Command::Version));
        let diff = Command::PrDiff {
            owner: "o".into(),
            repo: "r".into(),
            number: 12,
            patch: false,
        };
        assert_eq!(parse(&args(&["pr", "diff", "-R", "o/r", "12"])), Ok(diff.clone()));
        assert_eq!(
            parse(&args(&["pr", "diff", "12", "--repo=o/r", "--color", "never"])),
            Ok(diff)
        );
        assert!(matches!(
            parse(&args(&["pr", "diff", "--patch", "-R", "o/r", "12"])),
            Ok(Command::PrDiff { patch: true, .. })
        ));
        assert!(matches!(
            parse(&args(&["pr", "diff", "12"])),
            Err(ArgvError::UnsupportedCommand(_))
        ));
        assert!(matches!(
            parse(&args(&["pr", "diff", "-R", "o/r", "12", "--color", "pink"])),
            Err(ArgvError::InvalidValue { .. })
        ));
    }

    #[test]
    fn default_method_is_post_with_fields() {
        let req = api(&["api", "/repos/o/r/issues", "-f", "title=Hi", "-F", "draft=true"]);
        assert_eq!(req.method(), "POST");
        assert!(!req.fields_in_query());
        assert_eq!(api(&["api", "/user"]).method(), "GET");
        assert_eq!(api(&["api", "--input", "-", "/x"]).method(), "POST");
        assert_eq!(api(&["api", "graphql", "-f", "query=q"]).method(), "POST");
        assert_eq!(api(&["api", "-X", "get", "-F", "a=1", "/x"]).method(), "GET");
    }

    #[test]
    fn typed_vs_raw_fields() {
        let req = api(&[
            "api",
            "-X",
            "POST",
            "/x",
            "-F",
            "a=true",
            "-F",
            "b=false",
            "-F",
            "c=null",
            "-F",
            "d=42",
            "-F",
            "e=4.5",
            "-F",
            "f=text",
            "-F",
            "g=@body.txt",
            "-f",
            "h=true",
            "-f",
            "i=42",
            "-f",
            "j=@body.txt",
            "-F",
            "k[]=1",
            "-f",
            "k[]=two",
        ]);
        let mut reads = Vec::new();
        let typed = typed_fields(&req.fields, &mut |p| {
            reads.push(p.to_owned());
            Ok("from file".into())
        })
        .unwrap();
        assert_eq!(reads, ["body.txt"], "only -F reads @files");
        assert_eq!(
            json_body(&typed, false),
            json!({"a": true, "b": false, "c": null, "d": 42, "e": "4.5", "f": "text", "g": "from file",
                   "h": "true", "i": "42", "j": "@body.txt", "k": [1, "two"]})
        );
        // The same typed values on a GET become the strings gh would send.
        let pairs = query_pairs(&typed);
        assert_eq!(pairs[2], ("c".to_owned(), String::new()));
        assert_eq!(pairs[3], ("d".to_owned(), "42".to_owned()));
        assert_eq!(pairs[10], ("k[]".to_owned(), "1".to_owned()));
        assert_eq!(pairs[11], ("k[]".to_owned(), "two".to_owned()));
    }

    #[test]
    fn integer_fields_are_reformatted() {
        let typed = typed_fields(
            &[Field {
                key: "page".into(),
                value: "007".into(),
                raw: false,
            }],
            &mut no_files,
        )
        .unwrap();
        assert_eq!(query_pairs(&typed), [("page".to_owned(), "7".to_owned())]);
    }

    #[test]
    fn graphql_fields_go_under_variables() {
        let req = api(&["api", "graphql", "-f", "query=query($n: Int!) { x }", "-F", "n=3"]);
        let typed = typed_fields(&req.fields, &mut no_files).unwrap();
        assert_eq!(
            json_body(&typed, req.is_graphql()),
            json!({"query": "query($n: Int!) { x }", "variables": {"n": 3}})
        );
    }

    #[test]
    fn input_from_stdin() {
        let req = api(&["api", "-X", "PUT", "--input", "-", "/repos/o/r/pulls/1/merge"]);
        assert_eq!(req.input.as_deref(), Some("-"));
        assert_eq!(req.method(), "PUT");
    }

    #[test]
    fn fields_with_input_go_to_the_query() {
        let req = api(&["api", "-X", "PUT", "--input", "-", "-F", "a=1", "/x"]);
        assert!(req.fields_in_query());
    }

    #[test]
    fn misc_flag_spellings() {
        let req = api(&[
            "api",
            "-iXPATCH",
            "--header=Accept: application/vnd.github.raw",
            "--cache",
            "1h",
            "/x",
        ]);
        assert!(req.include);
        assert_eq!(req.method(), "PATCH");
        assert_eq!(req.accept(), Accept::Raw);
        assert_eq!(req.cache.as_deref(), Some("1h"));
        let req = api(&["api", "--paginate", "--slurp", "--hostname", "github.com", "/x"]);
        assert!(req.paginate && req.slurp);
        // The last Accept wins.
        let req = api(&[
            "api",
            "-H",
            "Accept: application/vnd.github.diff",
            "-H",
            "accept: application/json",
            "/x",
        ]);
        assert_eq!(req.accept(), Accept::Json);
    }

    #[test]
    fn unsupported_flags_and_commands() {
        assert_eq!(
            parse(&args(&["api", "--jq", ".x", "/x"])),
            Err(ArgvError::UnsupportedFlag("--jq".into()))
        );
        assert_eq!(
            parse(&args(&["api", "-p", "x", "/x"])),
            Err(ArgvError::UnsupportedFlag("-p".into()))
        );
        assert_eq!(
            parse(&args(&["api", "--template", "t", "/x"])),
            Err(ArgvError::UnsupportedFlag("--template".into()))
        );
        assert_eq!(
            parse(&args(&["api", "--verbose", "/x"])),
            Err(ArgvError::UnsupportedFlag("--verbose".into()))
        );
        // A real flag on the wrong command.
        assert_eq!(
            parse(&args(&["auth", "status", "-X", "GET"])),
            Err(ArgvError::UnsupportedFlag("-X".into()))
        );
        assert_eq!(
            parse(&args(&["repo", "view"])),
            Err(ArgvError::UnsupportedCommand("repo view".into()))
        );
        assert!(matches!(parse(&args(&["api"])), Err(ArgvError::UnsupportedCommand(_))));
        assert_eq!(
            parse(&args(&["api", "--hostname", "ghe.example.com", "/x"])),
            Err(ArgvError::UnsupportedHost("ghe.example.com".into()))
        );
        assert_eq!(
            parse(&args(&["api", "/x", "-X"])),
            Err(ArgvError::MissingValue("-X".into()))
        );
        assert!(matches!(
            parse(&args(&["api", "-F", "novalue", "/x"])),
            Err(ArgvError::InvalidValue { .. })
        ));
    }
}
