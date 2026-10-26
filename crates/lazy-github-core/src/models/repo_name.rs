//! `owner/name` repo names, as git remotes spell them.

use std::fmt;

/// `owner/name` of a GitHub repo, as parsed from a remote URL. `Display` writes `owner/name`.
///
/// The host isn't kept or checked, so GitHub Enterprise remotes parse the same way as github.com ones.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoName {
    owner: String,
    name: String,
}

/// The URL schemes git can fetch a GitHub repo over.
const NETWORK_SCHEMES: [&str; 6] = ["https", "http", "ssh", "git", "git+ssh", "ssh+git"];

impl RepoName {
    /// Parses a git remote URL (`git remote get-url` output) into the repo it names.
    ///
    /// Accepts `https`, `http`, `ssh`, `git`, `git+ssh` and `ssh+git` URLs (with or without user info and a port)
    /// and scp-like `[user@]host:owner/name` remotes. The path must be exactly `owner/name`, with an optional
    /// trailing `/` and then an optional `.git`; dots in `name` are kept. Local paths, `file://` URLs and any other
    /// path shape give `None`.
    pub fn from_remote_url(url: &str) -> Option<Self> {
        let url = url.trim();
        let path = url
            .split_once("://")
            .map_or_else(|| scp_like_path(url), |(scheme, rest)| url_path(scheme, rest))?;
        let path = path.strip_prefix('/').unwrap_or(path);
        let path = path.strip_suffix('/').unwrap_or(path);
        let path = path.strip_suffix(".git").unwrap_or(path);
        let (owner, name) = path.split_once('/')?;
        if owner.is_empty() || name.is_empty() || name.contains('/') {
            return None;
        }
        Some(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// The owner: a user or organization login.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The repo's name within its owner.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether `full_name` (an API `owner/name`) names this repo. GitHub names are case-insensitive (Py bug B42), so
    /// this ignores ASCII case.
    pub fn matches(&self, full_name: &str) -> bool {
        full_name.split_once('/').is_some_and(|(owner, name)| {
            owner.eq_ignore_ascii_case(&self.owner) && name.eq_ignore_ascii_case(&self.name)
        })
    }
}

/// The path of a `scheme://authority/path` URL, when the scheme is one git fetches GitHub repos over and there's a
/// host.
fn url_path<'a>(scheme: &str, rest: &'a str) -> Option<&'a str> {
    if !NETWORK_SCHEMES.iter().any(|known| scheme.eq_ignore_ascii_case(known)) {
        return None;
    }
    let (authority, path) = rest.split_at(rest.find('/')?);
    // Drop `userinfo@` (the last `@`, since user info may hold more) and `:port`.
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = host.split_once(':').map_or(host, |(host, _port)| host);
    (!host.is_empty()).then_some(path)
}

/// The path of an scp-like `[user@]host:path` remote: a `:` with no `/` before it. Like git, a one-letter "host"
/// followed by a slash is a Windows drive (`C:/repos/n.git`), so a local path.
fn scp_like_path(url: &str) -> Option<&str> {
    let (authority, path) = url.split_once(':')?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let drive = host.len() == 1 && host.chars().all(|c| c.is_ascii_alphabetic());
    let local = host.is_empty() || authority.contains('/') || (drive && path.starts_with(['/', '\\']));
    (!local).then_some(path)
}

impl fmt::Display for RepoName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn parsed(url: &str) -> Option<String> {
        RepoName::from_remote_url(url).map(|repo| repo.to_string())
    }

    // One case per row of #115's table, Rust column.
    #[rstest]
    #[case::scp("git@github.com:o/n.git", "o/n")]
    #[case::scp_no_suffix("git@github.com:o/n", "o/n")]
    #[case::scp_dotted_name("git@github.com:o/my.repo.git", "o/my.repo")]
    #[case::https("https://github.com/o/n.git", "o/n")]
    #[case::https_no_suffix("https://github.com/o/n", "o/n")]
    #[case::https_trailing_slash("https://github.com/o/n/", "o/n")]
    #[case::https_userinfo("https://user:token@github.com/o/n.git", "o/n")]
    #[case::https_port("https://github.com:443/o/n.git", "o/n")]
    #[case::ssh_url("ssh://git@github.com/o/n.git", "o/n")]
    #[case::ssh_url_port("ssh://git@github.com:22/o/n.git", "o/n")]
    #[case::http("http://github.com/o/n.git", "o/n")]
    #[case::git_protocol("git://github.com/o/n.git", "o/n")]
    #[case::enterprise_scp("git@ghe.example.com:o/n.git", "o/n")]
    #[case::enterprise_https("https://ghe.example.com/o/n.git", "o/n")]
    fn parses_github_remotes(#[case] url: &str, #[case] expected: &str) {
        assert_eq!(parsed(url).as_deref(), Some(expected), "{url}");
    }

    #[rstest]
    #[case::local_path("/srv/repos/n.git")]
    #[case::file_url("file:///srv/n.git")]
    #[case::relative_path("../n")]
    #[case::owner_only("https://github.com/o")]
    #[case::extra_segment("https://github.com/o/n/extra")]
    fn rejects_other_remotes(#[case] url: &str) {
        assert_eq!(parsed(url), None, "{url}");
    }

    #[rstest]
    #[case::trimmed("  git@github.com:o/n.git\n", Some("o/n"))]
    #[case::other_schemes("git+ssh://git@github.com/o/n.git", Some("o/n"))]
    #[case::scheme_case("HTTPS://github.com/o/n", Some("o/n"))]
    #[case::scp_leading_slash("git@github.com:/o/n.git", Some("o/n"))]
    #[case::suffix_then_slash("https://github.com/o/n.git/", Some("o/n"))]
    #[case::only_suffix("https://github.com/o/.git", None)]
    #[case::empty_owner("https://github.com//n", None)]
    #[case::no_host("https:///o/n", None)]
    #[case::no_path("https://github.com", None)]
    #[case::windows_drive("C:/repos/o/n.git", None)]
    #[case::windows_drive_backslash("C:\\repos\\n.git", None)]
    #[case::slash_before_colon("./o:n/x", None)]
    #[case::empty("", None)]
    fn edge_cases(#[case] url: &str, #[case] expected: Option<&str>) {
        assert_eq!(parsed(url).as_deref(), expected, "{url}");
    }

    #[test]
    fn accessors_and_display() {
        let repo = RepoName::from_remote_url("git@github.com:Octo-Org/my.repo.git").unwrap();
        assert_eq!(repo.owner(), "Octo-Org");
        assert_eq!(repo.name(), "my.repo");
        assert_eq!(repo.to_string(), "Octo-Org/my.repo");
    }

    #[test]
    fn matches_ignores_ascii_case() {
        let repo = RepoName::from_remote_url("https://github.com/Parity-Bot/DotFiles").unwrap();
        assert!(repo.matches("Parity-Bot/DotFiles"));
        assert!(repo.matches("parity-bot/dotfiles"));
        assert!(repo.matches("PARITY-BOT/DOTFILES"));
        assert!(!repo.matches("parity-bot/dotfiles2"));
        assert!(!repo.matches("other/dotfiles"));
        assert!(!repo.matches("parity-bot"));
        assert!(!repo.matches("parity-bot/dotfiles/extra"));
    }
}
