//! Building the `gh api` argument vector for one request.

use std::ffi::OsString;
use std::fmt;
use std::time::Duration;

/// The HTTP method of a [`GhRequest`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Method {
    /// `GET`, the default.
    #[default]
    Get,
    /// `POST`.
    Post,
    /// `PATCH`.
    Patch,
    /// `PUT`.
    Put,
    /// `DELETE`.
    Delete,
}

impl Method {
    /// The method name as `gh api -X` expects it.
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Patch => "PATCH",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The media type sent in the request's `Accept` header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Accept {
    /// `application/vnd.github+json`, the default.
    #[default]
    Json,
    /// `application/vnd.github.diff`, for pull request diffs.
    Diff,
    /// `application/vnd.github.raw`, for raw file contents such as READMEs.
    Raw,
    /// Any other media type, sent verbatim.
    Custom(String),
}

impl Accept {
    /// The header value for this media type.
    pub fn as_str(&self) -> &str {
        match self {
            Accept::Json => "application/vnd.github+json",
            Accept::Diff => "application/vnd.github.diff",
            Accept::Raw => "application/vnd.github.raw",
            Accept::Custom(value) => value,
        }
    }
}

/// One GitHub REST API request, run through `gh api`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GhRequest {
    /// The HTTP method.
    pub method: Method,
    /// The API path, such as `/user/repos`, without a query string.
    ///
    /// Passed to `gh` verbatim: callers must percent-encode any segment that can contain reserved characters (a
    /// branch name with `#` or `?`, a file path with spaces). Only `query` is encoded here.
    pub path: String,
    /// Query parameters, URL-encoded into the path in insertion order.
    pub query: Vec<(String, String)>,
    /// The `Accept` media type.
    pub accept: Accept,
    /// Extra request headers, sent after `Accept`.
    pub headers: Vec<(String, String)>,
    /// The JSON request body, written to `gh`'s stdin with `--input -`.
    pub body: Option<serde_json::Value>,
    /// How long `gh` may serve this response from its cache. Only applies to `GET`; ignored otherwise.
    pub cache: Option<Duration>,
    /// How long this one request may run before `gh` is killed, overriding the runner's limit for its kind (see
    /// [`GhTimeouts`](super::GhTimeouts)). For calls known to run long, such as log downloads.
    pub timeout: Option<Duration>,
}

impl GhRequest {
    /// A request with the given method and path and every other field defaulted.
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            ..Self::default()
        }
    }

    /// A `GET` request for `path`.
    pub fn get(path: impl Into<String>) -> Self {
        Self::new(Method::Get, path)
    }

    /// The path with the URL-encoded query string appended, if there is one.
    pub fn path_with_query(&self) -> String {
        if self.query.is_empty() {
            return self.path.clone();
        }
        let query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&self.query)
            .finish();
        format!("{}?{query}", self.path)
    }

    /// The arguments passed to `gh` for this request (not including the program name).
    ///
    /// `paginate` switches to `gh api --paginate`, which drops `-i`: the output is then the concatenated page bodies.
    pub fn argv(&self, hostname: Option<&str>, paginate: bool) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["api".into()];
        if !paginate {
            args.push("-i".into());
        }
        args.push("-X".into());
        args.push(self.method.as_str().into());
        if let Some(hostname) = hostname {
            args.push("--hostname".into());
            args.push(hostname.into());
        }
        if self.method == Method::Get
            && let Some(ttl) = self.cache
            && ttl.as_secs() > 0
        {
            args.push("--cache".into());
            args.push(format!("{}s", ttl.as_secs()).into());
        }
        if paginate {
            args.push("--paginate".into());
        }
        args.push("-H".into());
        args.push(format!("Accept: {}", self.accept.as_str()).into());
        for (name, value) in &self.headers {
            args.push("-H".into());
            args.push(format!("{name}: {value}").into());
        }
        if self.body.is_some() {
            args.push("--input".into());
            args.push("-".into());
        }
        args.push(self.path_with_query().into());
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn argv(req: &GhRequest, hostname: Option<&str>, paginate: bool) -> Vec<String> {
        req.argv(hostname, paginate)
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect()
    }

    #[test]
    fn plain_get() {
        let req = GhRequest::get("/user");
        assert_eq!(
            argv(&req, None, false),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github+json",
                "/user"
            ]
        );
    }

    #[test]
    fn get_with_query_is_encoded_in_order() {
        let mut req = GhRequest::get("/search/issues");
        req.query = vec![
            ("q".into(), "is:open repo:cli/cli".into()),
            ("sort".into(), "a+b".into()),
            ("page".into(), "1".into()),
        ];
        assert_eq!(
            argv(&req, None, false),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github+json",
                "/search/issues?q=is%3Aopen+repo%3Acli%2Fcli&sort=a%2Bb&page=1"
            ]
        );
    }

    #[test]
    fn get_with_cache() {
        let mut req = GhRequest::get("/user");
        req.cache = Some(Duration::from_mins(10));
        assert_eq!(
            argv(&req, None, false),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--cache",
                "600s",
                "-H",
                "Accept: application/vnd.github+json",
                "/user"
            ]
        );
    }

    #[test]
    fn zero_cache_is_omitted() {
        let mut req = GhRequest::get("/user");
        req.cache = Some(Duration::ZERO);
        assert!(!argv(&req, None, false).contains(&"--cache".to_owned()));
    }

    #[test]
    fn post_with_body_reads_stdin_and_never_caches() {
        let mut req = GhRequest::new(Method::Post, "/repos/o/r/issues");
        req.body = Some(json!({"title": "hi"}));
        req.cache = Some(Duration::from_mins(10));
        req.headers = vec![("X-Custom".into(), "yes".into())];
        assert_eq!(
            argv(&req, None, false),
            [
                "api",
                "-i",
                "-X",
                "POST",
                "-H",
                "Accept: application/vnd.github+json",
                "-H",
                "X-Custom: yes",
                "--input",
                "-",
                "/repos/o/r/issues"
            ]
        );
    }

    #[test]
    fn diff_accept() {
        let mut req = GhRequest::get("/repos/o/r/pulls/1");
        req.accept = Accept::Diff;
        assert_eq!(
            argv(&req, None, false),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "-H",
                "Accept: application/vnd.github.diff",
                "/repos/o/r/pulls/1"
            ]
        );
    }

    #[test]
    fn hostname() {
        let req = GhRequest::get("/user");
        assert_eq!(
            argv(&req, Some("github.example.com"), false),
            [
                "api",
                "-i",
                "-X",
                "GET",
                "--hostname",
                "github.example.com",
                "-H",
                "Accept: application/vnd.github+json",
                "/user"
            ]
        );
    }

    #[test]
    fn paginated_get_drops_include() {
        let mut req = GhRequest::get("/user/repos");
        req.query = vec![("per_page".into(), "50".into())];
        req.cache = Some(Duration::from_hours(24));
        assert_eq!(
            argv(&req, None, true),
            [
                "api",
                "-X",
                "GET",
                "--cache",
                "86400s",
                "--paginate",
                "-H",
                "Accept: application/vnd.github+json",
                "/user/repos?per_page=50"
            ]
        );
    }
}
