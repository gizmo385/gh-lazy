//! The debug info shared by `lazy-github debug`, the Debug Info modal and the bug report: what LazyGithub runs on,
//! then the config as it's saved.

use crate::config::Config;

/// Shown for anything that couldn't be found out.
const UNKNOWN: &str = "unknown";

/// What the debug info's "Library Versions" list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugEnv {
    /// LazyGithub's version.
    pub version: &'static str,
    /// What `rustc --version` printed at build time, after `rustc `, or `unknown`.
    pub rustc: &'static str,
    /// The installed gh's version ([`crate::github::Github::gh_version`]), or `None` when gh can't run or prints
    /// something unexpected.
    pub gh: Option<String>,
    /// The OS and the CPU architecture, such as `macos aarch64`.
    pub os: String,
    /// The terminal, from `TERM_PROGRAM`, `TERM_PROGRAM_VERSION` and `TERM`, or `unknown`.
    pub terminal: String,
}

impl DebugEnv {
    /// This build and machine. `env` returns an environment variable's value; `gh` is the gh version, if known.
    pub fn detect(env: impl Fn(&str) -> Option<String>, gh: Option<String>) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            rustc: env!("LAZY_GITHUB_RUSTC_VERSION"),
            gh,
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            terminal: terminal(env),
        }
    }
}

/// `TERM_PROGRAM`, then `TERM_PROGRAM_VERSION`, then `TERM` in parentheses, leaving out whichever are unset or empty:
/// `WezTerm 20240203-110809 (xterm-256color)`. Just `TERM` without parentheses when there's no `TERM_PROGRAM`.
fn terminal(env: impl Fn(&str) -> Option<String>) -> String {
    let var = |name| env(name).filter(|value| !value.is_empty());
    match (var("TERM_PROGRAM"), var("TERM")) {
        (Some(program), term) => {
            let mut text = program;
            if let Some(version) = var("TERM_PROGRAM_VERSION") {
                text.push(' ');
                text.push_str(&version);
            }
            if let Some(term) = term {
                text.push_str(" (");
                text.push_str(&term);
                text.push(')');
            }
            text
        }
        (None, Some(term)) => term,
        (None, None) => UNKNOWN.to_owned(),
    }
}

/// The debug info as markdown: the "Library Versions" list from `env`, then `config` as it's saved, in a `json` code
/// block. No trailing newline.
pub fn debug_markdown(env: &DebugEnv, config: &Config) -> String {
    format!(
        "# Library Versions\n\
         \n\
         - LazyGithub Version: {}\n\
         - Rustc Version: {}\n\
         - Gh Version: {}\n\
         - OS: {}\n\
         - Terminal: {}\n\
         \n\
         # LazyGithub Config\n\
         \n\
         ```json\n\
         {}\n\
         ```",
        env.version,
        env.rustc,
        env.gh.as_deref().unwrap_or(UNKNOWN),
        env.os,
        env.terminal,
        config.to_json_pretty(),
    )
}

/// The bug report's issue body, with `debug_markdown` (from [`debug_markdown`]) under "**Debug Info:**".
pub fn bug_report_template(debug_markdown: &str) -> String {
    format!(
        "**Describe the bug**\n\
         \n\
         **To Reproduce**\n\
         \n\
         **Expected behavior**\n\
         \n\
         **Installation Mechanism:** [Github CLI, nix, PyPi, etc]\n\
         \n\
         **Debug Info:**\n\
         {debug_markdown}\n\
         \n\
         **Additional context**"
    )
}

#[cfg(test)]
mod tests {
    use super::{DebugEnv, bug_report_template, debug_markdown, terminal};
    use crate::config::{Config, ConfigPaths};

    fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    fn fixed_env() -> DebugEnv {
        DebugEnv {
            version: "0.1.0",
            rustc: "1.97.0 (0123456789 2026-10-01)",
            gh: Some("2.89.0 (fake-gh)".to_owned()),
            os: "linux x86_64".to_owned(),
            terminal: "WezTerm 20240203-110809 (xterm-256color)".to_owned(),
        }
    }

    fn full_config() -> Config {
        let dir = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::in_dir(dir.path());
        std::fs::write(&paths.file, include_str!("../tests/fixtures/config/python-full.json")).unwrap();
        Config::load(&paths).unwrap()
    }

    #[test]
    fn terminal_with_program_version_and_term() {
        let vars = [
            ("TERM_PROGRAM", "WezTerm"),
            ("TERM_PROGRAM_VERSION", "20240203-110809"),
            ("TERM", "xterm-256color"),
        ];
        assert_eq!(terminal(env(&vars)), "WezTerm 20240203-110809 (xterm-256color)");
        assert_eq!(
            terminal(env(&[("TERM_PROGRAM", "Apple_Terminal"), ("TERM", "xterm-256color")])),
            "Apple_Terminal (xterm-256color)"
        );
    }

    #[test]
    fn terminal_with_only_term_program() {
        assert_eq!(terminal(env(&[("TERM_PROGRAM", "vscode")])), "vscode");
        assert_eq!(
            terminal(env(&[("TERM_PROGRAM", "vscode"), ("TERM_PROGRAM_VERSION", "1.95.0")])),
            "vscode 1.95.0"
        );
    }

    #[test]
    fn terminal_with_only_term() {
        assert_eq!(terminal(env(&[("TERM", "xterm-256color")])), "xterm-256color");
        // A version without its program says nothing on its own.
        assert_eq!(
            terminal(env(&[("TERM_PROGRAM_VERSION", "3.4"), ("TERM", "screen")])),
            "screen"
        );
    }

    #[test]
    fn terminal_with_neither() {
        assert_eq!(terminal(env(&[])), "unknown");
        assert_eq!(terminal(env(&[("TERM_PROGRAM", ""), ("TERM", "")])), "unknown");
    }

    #[test]
    fn detect_fills_in_this_build() {
        let detected = DebugEnv::detect(env(&[("TERM", "dumb")]), None);
        assert_eq!(detected.version, env!("CARGO_PKG_VERSION"));
        assert!(
            !detected.rustc.is_empty() && !detected.rustc.starts_with("rustc"),
            "{}",
            detected.rustc
        );
        assert_eq!(detected.gh, None);
        assert_eq!(
            detected.os,
            format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
        );
        assert_eq!(detected.terminal, "dumb");
    }

    #[test]
    fn an_unknown_gh_says_so() {
        let env = DebugEnv {
            gh: None,
            ..fixed_env()
        };
        let markdown = debug_markdown(&env, &full_config());
        assert!(markdown.contains("\n- Gh Version: unknown\n"), "{markdown}");
    }

    // The default config's paths are under the config folder, which is spelled with `\` on Windows.
    #[cfg(not(windows))]
    #[test]
    fn debug_markdown_with_the_default_config() {
        let config = Config::default_in(&ConfigPaths::in_dir("/home/lazy/.config/lazy-github"));
        insta::assert_snapshot!(debug_markdown(&fixed_env(), &config));
    }

    #[test]
    fn debug_markdown_with_a_full_config() {
        insta::assert_snapshot!(debug_markdown(&fixed_env(), &full_config()));
    }

    #[test]
    fn bug_report_template_around_the_debug_markdown() {
        insta::assert_snapshot!(bug_report_template(&debug_markdown(&fixed_env(), &full_config())));
    }

    #[test]
    fn debug_markdown_has_no_trailing_newline() {
        let markdown = debug_markdown(&fixed_env(), &full_config());
        assert!(markdown.ends_with("}\n```"), "{markdown}");
        let template = bug_report_template("X");
        assert!(
            template.ends_with("**Debug Info:**\nX\n\n**Additional context**"),
            "{template}"
        );
    }
}
