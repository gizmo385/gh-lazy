//! GitHub's auto-linked references in markdown bodies, made into real links.
//!
//! GitHub links `#123`, `owner/repo#123` and `@user` when it renders an issue, pull request or comment body, but the
//! raw markdown has no links there. [`expand`] rewrites them as markdown links before the body is rendered, so they
//! show and work as links. Python's `lib/github/references.py`.

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// `#123` or `owner/repo#123`. Python also requires the character before it not to be a word character, `/`, `&` or
/// `#` (no references inside identifiers, paths, HTML entities or `##headings`); the `regex` crate has no
/// lookbehind, so [`replace_unpreceded`] checks that.
static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    // A literal pattern, checked by the tests.
    Regex::new(r"(?:(?P<owner>[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)/(?P<repo>[A-Za-z0-9._-]+))?#(?P<number>\d+)\b")
        .expect("a valid reference pattern")
});

/// `@user`. The character before it may not be a word character, `@` or `/` (no emails, `@@chains` or paths).
static MENTION: LazyLock<Regex> = LazyLock::new(|| {
    // A literal pattern, checked by the tests.
    Regex::new(r"@(?P<username>[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)\b").expect("a valid mention pattern")
});

/// A line that opens or closes a fenced code block.
static FENCE: LazyLock<Regex> =
    // A literal pattern, checked by the tests.
    LazyLock::new(|| Regex::new(r"^\s*(?:```|~~~)").expect("a valid fence pattern"));

/// An inline code span.
static INLINE_CODE: LazyLock<Regex> =
    // A literal pattern, checked by the tests.
    LazyLock::new(|| Regex::new(r"`+[^`\n]*`+").expect("a valid inline code pattern"));

/// `body` with every `#N`, `owner/repo#N` and `@user` outside code made a markdown link: `#N` to issue `N` of
/// `repo_full_name` (`owner/name`), `owner/repo#N` to issue `N` of that repo (both as
/// `https://github.com/<owner>/<repo>/issues/<n>`, which GitHub redirects for pull requests), and `@user` to
/// `https://github.com/<user>`. Lines inside fenced code blocks and inline code spans are left alone.
///
/// A `repo_full_name` without an owner and a name leaves `body` as it is.
pub fn expand(body: &str, repo_full_name: &str) -> String {
    let Some((owner, repo)) = repo_full_name.split_once('/') else {
        return body.to_owned();
    };
    if owner.is_empty() || repo.is_empty() {
        return body.to_owned();
    }
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    // Python splits on every line boundary `str.splitlines` knows; GitHub bodies only use `\n` and `\r\n`.
    for line in body.split_inclusive('\n') {
        if FENCE.is_match(line) {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }
        let mut copied = 0;
        for code in INLINE_CODE.find_iter(line) {
            out.push_str(&expand_segment(&line[copied..code.start()], owner, repo));
            out.push_str(code.as_str());
            copied = code.end();
        }
        out.push_str(&expand_segment(&line[copied..], owner, repo));
    }
    out
}

/// One stretch of text outside code: mentions first, then references, as Python does.
fn expand_segment(segment: &str, owner: &str, repo: &str) -> String {
    let mentioned = replace_unpreceded(
        segment,
        &MENTION,
        |c| is_word(c) || c == '@' || c == '/',
        |caps| {
            let user = &caps["username"];
            format!("[@{user}](https://github.com/{user})")
        },
    );
    replace_unpreceded(
        &mentioned,
        &REFERENCE,
        |c| is_word(c) || matches!(c, '/' | '&' | '#'),
        |caps| {
            let owner = caps.name("owner").map_or(owner, |m| m.as_str());
            let repo = caps.name("repo").map_or(repo, |m| m.as_str());
            let number = &caps["number"];
            format!("[{}](https://github.com/{owner}/{repo}/issues/{number})", &caps[0])
        },
    )
}

/// Python's `\w`.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `re.sub` of `pattern` prefixed with a negative lookbehind: matches whose preceding character is `blocked` are
/// skipped, and the search goes on from the next character, as a lookbehind failing at one position does.
fn replace_unpreceded(
    text: &str,
    pattern: &Regex,
    blocked: impl Fn(char) -> bool,
    replace: impl Fn(&Captures<'_>) -> String,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while let Some(caps) = pattern.captures_at(text, at) {
        let Some(found) = caps.get(0) else { break };
        if text[..found.start()].chars().next_back().is_some_and(&blocked) {
            // Every match starts with an ASCII character, so the next character is one byte on.
            at = found.start() + 1;
            continue;
        }
        out.push_str(&text[copied..found.start()]);
        out.push_str(&replace(&caps));
        copied = found.end();
        at = found.end();
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn x(body: &str) -> String {
        expand(body, "parity-bot/pr-details")
    }

    #[test]
    fn issue_references() {
        assert_eq!(
            x("see #3."),
            "see [#3](https://github.com/parity-bot/pr-details/issues/3)."
        );
        assert_eq!(x("#3"), "[#3](https://github.com/parity-bot/pr-details/issues/3)");
        assert_eq!(
            x("and parity-bot/dotfiles#12, too"),
            "and [parity-bot/dotfiles#12](https://github.com/parity-bot/dotfiles/issues/12), too"
        );
        assert_eq!(x("(#7)"), "([#7](https://github.com/parity-bot/pr-details/issues/7))");
    }

    #[test]
    fn mentions() {
        assert_eq!(
            x("Thanks @parity-bot!"),
            "Thanks [@parity-bot](https://github.com/parity-bot)!"
        );
        assert_eq!(x("@a"), "[@a](https://github.com/a)");
    }

    #[test]
    fn nothing_inside_code() {
        let fenced = "```\n#3 and @x\n```\nafter #4\n";
        assert_eq!(
            x(fenced),
            "```\n#3 and @x\n```\nafter [#4](https://github.com/parity-bot/pr-details/issues/4)\n"
        );
        assert_eq!(x("~~~\n@x\n~~~\n"), "~~~\n@x\n~~~\n");
        assert_eq!(
            x("`#3` but #3 and ``@x``"),
            "`#3` but [#3](https://github.com/parity-bot/pr-details/issues/3) and ``@x``"
        );
    }

    #[test]
    fn preceding_characters_that_block() {
        for body in [
            "##heading",
            "##3",
            "a&#3",
            "path/#3",
            "abc#3",
            "user@example.com",
            "@@x",
            "a/@x",
            "#3a",
            "_@x",
        ] {
            assert_eq!(x(body), body, "{body}");
        }
        // The next position still matches, as a lookbehind failing at one position doesn't stop the search.
        assert_eq!(x("x-#3"), "x-[#3](https://github.com/parity-bot/pr-details/issues/3)");
    }

    #[test]
    fn a_repo_without_owner_and_name_leaves_the_body() {
        assert_eq!(expand("#3 @x", "nameless"), "#3 @x");
        assert_eq!(expand("#3", "/name"), "#3");
        assert_eq!(expand("#3", "owner/"), "#3");
        assert_eq!(expand("", "o/r"), "");
    }

    #[test]
    fn non_ascii_text_around_references() {
        assert_eq!(
            x("é #3 ü"),
            "é [#3](https://github.com/parity-bot/pr-details/issues/3) ü"
        );
        // `é` is a word character, as in Python.
        assert_eq!(x("é#3 é@x"), "é#3 é@x");
    }

    #[test]
    fn the_body_of_pr_5() {
        let body = "Adds a cursor to the viewer, as discussed in #3.\n\n- `j` and `k` move it\n- the current line is \
                    marked with `>`\n\nThanks @parity-bot for the first review!\n";
        assert_eq!(
            x(body),
            "Adds a cursor to the viewer, as discussed in [#3](https://github.com/parity-bot/pr-details/issues/3).\n\n\
             - `j` and `k` move it\n- the current line is marked with `>`\n\n\
             Thanks [@parity-bot](https://github.com/parity-bot) for the first review!\n"
        );
    }
}
