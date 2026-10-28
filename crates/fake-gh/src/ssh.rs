//! `fake-ssh`: the SSH transport for git in parity runs. Git runs it in place of `ssh` (`GIT_SSH`, with
//! `GIT_SSH_VARIANT=simple`) to reach `git@github.com:<owner>/<name>.git`, and it serves the request from a local bare
//! repo under `FAKE_SSH_ROOT` instead. The behavior is documented in `parity/README.md` ("The isolated environment").

use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::log::{LockedLog, LogEntry, SSH_COMMAND};

/// The exit code for every request fake-ssh refuses, as ssh and GitHub use for a failed remote command.
pub const EXIT_REFUSED: u8 = 128;
/// What GitHub answers for a repo it doesn't have (or won't show).
pub const NOT_FOUND: &str = "ERROR: Repository not found.";

/// The git services fake-ssh runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    /// Fetches and `ls-remote`.
    UploadPack,
    /// Pushes.
    ReceivePack,
}

impl Service {
    /// The name git asks for, e.g. `git-upload-pack`.
    pub fn name(self) -> &'static str {
        match self {
            Service::UploadPack => "git-upload-pack",
            Service::ReceivePack => "git-receive-pack",
        }
    }

    /// The `git` subcommand that serves it.
    fn subcommand(self) -> &'static str {
        match self {
            Service::UploadPack => "upload-pack",
            Service::ReceivePack => "receive-pack",
        }
    }
}

/// fake-ssh's configuration, from the environment.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `FAKE_SSH_ROOT`: the directory repo paths resolve under.
    pub root: Option<PathBuf>,
    /// `FAKE_GH_LOG`: the request log shared with fake-gh.
    pub log: Option<PathBuf>,
}

impl Env {
    pub fn from_process() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        Env {
            root: var("FAKE_SSH_ROOT"),
            log: var("FAKE_GH_LOG"),
        }
    }
}

/// A refused request: the message for stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

/// The remote command from fake-ssh's arguments: `[-p <port>] <host> <command>`, the host and port ignored.
pub fn remote_command(args: &[String]) -> Result<&str, Refusal> {
    let rest = match args {
        [flag, _port, rest @ ..] if flag == "-p" => rest,
        rest => rest,
    };
    match rest {
        [_host, command] => Ok(command),
        _ => Err(Refusal(format!(
            "fake-ssh: usage: fake-ssh [-p <port>] <host> <command>, got {args:?}"
        ))),
    }
}

/// Splits a remote command, `git-upload-pack '<path>'`, into its service and its path with the quotes and one
/// leading `/` stripped.
pub fn parse_command(command: &str) -> Result<(Service, String), Refusal> {
    let unsupported = || Refusal(format!("fake-ssh: unsupported command: {command}"));
    let (name, quoted) = command.split_once(' ').ok_or_else(unsupported)?;
    let service = match name {
        "git-upload-pack" => Service::UploadPack,
        "git-receive-pack" => Service::ReceivePack,
        _ => return Err(unsupported()),
    };
    // Git quotes the path with `'`, escaping a `'` inside it as `'\''`. Repo paths here never hold one.
    let path = quoted
        .strip_prefix('\'')
        .and_then(|p| p.strip_suffix('\''))
        .filter(|p| !p.contains('\''))
        .ok_or_else(unsupported)?;
    let path = path.strip_prefix('/').unwrap_or(path);
    Ok((service, path.to_owned()))
}

/// The repo directory `path` names under `root`. Anything that isn't an existing directory inside `root` (after
/// following `..` and symlinks) is [`NOT_FOUND`].
pub fn resolve(root: &Path, path: &str) -> Result<PathBuf, Refusal> {
    let not_found = || Refusal(NOT_FOUND.to_owned());
    let relative = Path::new(path);
    // An absolute or drive-prefixed path would replace `root` in `join`.
    if path.is_empty()
        || relative
            .components()
            .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    {
        return Err(not_found());
    }
    let root = root.canonicalize().map_err(|_| not_found())?;
    let dir = root.join(relative).canonicalize().map_err(|_| not_found())?;
    if dir == root || !dir.starts_with(&root) || !dir.is_dir() {
        return Err(not_found());
    }
    Ok(dir)
}

/// Runs fake-ssh: serves the request with `git upload-pack` / `git receive-pack`, its standard streams this
/// process's, logs it and returns the exit code. A refused request writes its message to `stderr`.
pub fn run(args: &[String], env: &Env, stderr: &mut dyn Write) -> u8 {
    let mut entry = LogEntry {
        argv: args.to_vec(),
        command: Some(SSH_COMMAND.to_owned()),
        ..LogEntry::default()
    };
    let code = match serve(args, env, &mut entry) {
        Ok(code) => code,
        Err(Refusal(message)) => {
            let _ = writeln!(stderr, "{message}");
            EXIT_REFUSED
        }
    };
    entry.status = Some(u16::from(code));
    match log(env, entry) {
        Ok(()) => code,
        Err(e) => {
            let _ = writeln!(stderr, "fake-ssh: {e}");
            EXIT_REFUSED
        }
    }
}

fn serve(args: &[String], env: &Env, entry: &mut LogEntry) -> Result<u8, Refusal> {
    let (service, path) = parse_command(remote_command(args)?)?;
    entry.service = Some(service.name().to_owned());
    entry.path = Some(path.clone());
    let root = env
        .root
        .as_deref()
        .ok_or_else(|| Refusal("fake-ssh: FAKE_SSH_ROOT is not set".to_owned()))?;
    let dir = resolve(root, &path)?;
    // No shell, and stdin, stdout and stderr are inherited: git on the other end speaks the pack protocol with it.
    let status = Command::new("git")
        .arg(service.subcommand())
        .arg(&dir)
        .status()
        .map_err(|e| Refusal(format!("fake-ssh: can't run git {}: {e}", service.subcommand())))?;
    // A git killed by a signal has no code; report it as refused.
    Ok(status.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(EXIT_REFUSED))
}

fn log(env: &Env, entry: LogEntry) -> io::Result<()> {
    match &env.log {
        Some(path) => LockedLog::open(path)?.append(entry).map(|_| ()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_remote_command_is_the_last_of_two_arguments_after_an_optional_port() {
        let cmd = "git-upload-pack 'o/n.git'";
        assert_eq!(remote_command(&args(&["git@github.com", cmd])), Ok(cmd));
        assert_eq!(remote_command(&args(&["-p", "22", "git@github.com", cmd])), Ok(cmd));
        assert!(remote_command(&args(&["git@github.com"])).is_err());
        assert!(remote_command(&args(&["-o", "SendEnv=GIT_PROTOCOL", "git@github.com", cmd])).is_err());
    }

    #[test]
    fn commands_parse_into_a_service_and_an_unquoted_path() {
        assert_eq!(
            parse_command("git-upload-pack 'parity-bot/pr-details.git'"),
            Ok((Service::UploadPack, "parity-bot/pr-details.git".to_owned()))
        );
        assert_eq!(
            parse_command("git-receive-pack '/o/n.git'"),
            Ok((Service::ReceivePack, "o/n.git".to_owned()))
        );
        for bad in [
            "git-upload-archive 'o/n.git'",
            "rm -rf /",
            "git-upload-pack",
            "git-upload-pack o/n.git",
        ] {
            let Err(Refusal(message)) = parse_command(bad) else {
                panic!("{bad:?} should be refused");
            };
            assert!(message.starts_with("fake-ssh: unsupported command"), "{message}");
        }
    }

    #[test]
    fn paths_resolve_only_to_directories_inside_the_root() {
        let dir = TempDir::new();
        let root = dir.path().join("remotes");
        std::fs::create_dir_all(root.join("o/n.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("outside.git")).unwrap();
        assert_eq!(
            resolve(&root, "o/n.git").unwrap(),
            root.join("o/n.git").canonicalize().unwrap()
        );
        for bad in [
            "o/missing.git",
            "../outside.git",
            "o/../../outside.git",
            "",
            ".",
            "/o/n.git",
        ] {
            assert_eq!(
                resolve(&root, bad),
                Err(Refusal(NOT_FOUND.to_owned())),
                "{bad:?} should not resolve"
            );
        }
    }

    #[test]
    fn a_refused_request_is_logged_with_its_exit_code() {
        let dir = TempDir::new();
        let log = dir.path().join("requests.jsonl");
        let env = Env {
            root: Some(dir.path().to_owned()),
            log: Some(log.clone()),
        };
        let mut stderr = Vec::new();
        let argv = args(&["git@github.com", "git-receive-pack 'o/missing.git'"]);
        assert_eq!(run(&argv, &env, &mut stderr), EXIT_REFUSED);
        assert_eq!(String::from_utf8(stderr).unwrap(), format!("{NOT_FOUND}\n"));
        let text = std::fs::read_to_string(&log).unwrap();
        let line: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(
            line,
            serde_json::json!({
                "seq": 1,
                "argv": argv,
                "command": "ssh",
                "service": "git-receive-pack",
                "method": null,
                "path": "o/missing.git",
                "query": null,
                "accept": null,
                "cache": null,
                "cache_control": null,
                "body": null,
                "fixture": null,
                "status": 128,
                "miss": false,
                "error": null,
            })
        );
    }

    #[test]
    fn an_unsupported_command_is_logged_without_a_service() {
        let dir = TempDir::new();
        let log = dir.path().join("requests.jsonl");
        let env = Env {
            root: Some(dir.path().to_owned()),
            log: Some(log.clone()),
        };
        let mut stderr = Vec::new();
        assert_eq!(
            run(&args(&["h", "git-upload-archive 'o/n.git'"]), &env, &mut stderr),
            EXIT_REFUSED
        );
        let text = String::from_utf8(stderr).unwrap();
        assert!(text.starts_with("fake-ssh: unsupported command"), "{text}");
        let entries = crate::log::RequestLog::parse(&std::fs::read_to_string(&log).unwrap());
        let [entry] = entries.entries() else {
            panic!("one line expected");
        };
        assert!(entry.is_ssh() && entry.service.is_none() && entry.path.is_none());
        assert_eq!(entry.status, Some(128));
    }
}
