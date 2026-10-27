//! The effect runner: performs `Effect`s in the background and sends their results back as `Event`s.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;

use indexmap::IndexMap;
use lazy_github_core::cache;
use lazy_github_core::config::{Config, ConfigPaths};
use lazy_github_core::git::GitCli;
use lazy_github_core::github::Github;
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::{Instant, MissedTickBehavior};

use crate::effect::{Effect, RequestId, TimerId};
use crate::event::Event;

/// Runs effects. Every method returns at once; the work happens on spawned tasks.
pub struct Runtime<G: Github> {
    gh: G,
    git: GitCli,
    tx: mpsc::UnboundedSender<Event>,
    /// GitHub and git requests alike, so `Effect::Cancel` works for both.
    tasks: HashMap<RequestId, AbortHandle>,
    timers: HashMap<TimerId, AbortHandle>,
    saver: ConfigSaver,
    cache_writer: CacheWriter,
    /// The terminal, for the bell and the clipboard.
    terminal: Box<dyn Write + Send>,
}

impl<G: Github> Runtime<G> {
    /// A runtime that asks GitHub through `gh` and the user's working directory through `git`, sends results to
    /// `tx` and saves the config to `paths`. Must be called inside a tokio runtime.
    pub fn new(gh: G, git: GitCli, tx: mpsc::UnboundedSender<Event>, paths: ConfigPaths) -> Self {
        let saver = ConfigSaver::spawn(paths, tx.clone());
        let cache_writer = CacheWriter::spawn();
        Self {
            gh,
            git,
            tx,
            tasks: HashMap::new(),
            timers: HashMap::new(),
            saver,
            cache_writer,
            terminal: Box::new(io::stdout()),
        }
    }

    /// Sends the bell and the clipboard to `terminal` instead of stdout.
    #[cfg(test)]
    fn with_terminal(mut self, terminal: impl Write + Send + 'static) -> Self {
        self.terminal = Box::new(terminal);
        self
    }

    /// Starts performing `effect`. Never blocks.
    pub fn execute(&mut self, effect: Effect) {
        // Forget finished tasks so the maps only hold running ones.
        self.tasks.retain(|_, task| !task.is_finished());
        self.timers.retain(|_, timer| !timer.is_finished());
        match effect {
            Effect::Github { id, request } => {
                let gh = self.gh.clone();
                let tx = self.tx.clone();
                let task = tokio::spawn(async move {
                    let response = request.run(&gh).await;
                    // The loop has stopped if this fails, and then nobody wants the response.
                    let _ = tx.send(Event::Github { id, response });
                });
                self.tasks.insert(id, task.abort_handle());
            }
            Effect::Git { id, request } => {
                let git = self.git.clone();
                let tx = self.tx.clone();
                let task = tokio::spawn(async move {
                    let response = request.run(&git).await;
                    let _ = tx.send(Event::Git { id, response });
                });
                self.tasks.insert(id, task.abort_handle());
            }
            Effect::Cancel(id) => {
                // Aborting drops the future, and with it the `gh` or `git` child (`kill_on_drop`).
                if let Some(task) = self.tasks.remove(&id) {
                    task.abort();
                }
            }
            Effect::SaveConfig(config) => self.saver.save(config),
            Effect::ReadCache { id, path } => {
                let tx = self.tx.clone();
                let task = tokio::spawn(async move {
                    let result = match tokio::task::spawn_blocking(move || cache::read(&path)).await {
                        Ok(read) => read.map_err(|e| e.to_string()),
                        Err(e) => Err(format!("the read task failed: {e}")),
                    };
                    let _ = tx.send(Event::CacheRead { id, result });
                });
                self.tasks.insert(id, task.abort_handle());
            }
            Effect::WriteCache { path, text } => self.cache_writer.write(path, text),
            Effect::StartInterval { id, every } => {
                if self.timers.contains_key(&id) {
                    return;
                }
                let tx = self.tx.clone();
                let task = tokio::spawn(async move {
                    let mut interval = tokio::time::interval_at(Instant::now() + every, every);
                    // After a suspend, tick once rather than in a burst.
                    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
                    loop {
                        interval.tick().await;
                        if tx.send(Event::Timer(id)).is_err() {
                            break;
                        }
                    }
                });
                self.timers.insert(id, task.abort_handle());
            }
            Effect::StartTimeout { id, after } => {
                let tx = self.tx.clone();
                let task = tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = tx.send(Event::Timer(id));
                });
                if let Some(old) = self.timers.insert(id, task.abort_handle()) {
                    old.abort();
                }
            }
            Effect::CancelTimer(id) => {
                if let Some(timer) = self.timers.remove(&id) {
                    timer.abort();
                }
            }
            // One byte between frames, on the thread that draws them, so it can't land inside a frame's escapes.
            Effect::Bell => {
                if let Err(err) = self.terminal.write_all(b"\x07").and_then(|()| self.terminal.flush()) {
                    tracing::debug!("Failed to ring the bell: {err}");
                }
            }
            // Between frames too, for the same reason.
            Effect::SetClipboard(text) => {
                if let Err(err) = self
                    .terminal
                    .write_all(&osc52(&text))
                    .and_then(|()| self.terminal.flush())
                {
                    tracing::debug!("Failed to write the clipboard: {err}");
                }
            }
            // `App` already stopped the loop; there's nothing to perform.
            Effect::Quit => {}
        }
    }

    /// Starts performing each effect, in order.
    pub fn execute_all(&mut self, effects: impl IntoIterator<Item = Effect>) {
        for effect in effects {
            self.execute(effect);
        }
    }

    /// Aborts every request (killing their `gh` and `git` children) and timer, then waits for the config saver to write the
    /// newest pending save and the cache writer to write every pending file.
    pub async fn shutdown(self) {
        for task in self.tasks.into_values().chain(self.timers.into_values()) {
            task.abort();
        }
        self.saver.finish().await;
        self.cache_writer.finish().await;
    }
}

/// The OSC 52 sequence that asks the terminal to put `text` on the system clipboard: `ESC ] 52 ; c ; <base64> BEL`,
/// as Textual's `App.copy_to_clipboard` writes it.
fn osc52(text: &str) -> Vec<u8> {
    let mut out = b"\x1b]52;c;".to_vec();
    out.extend_from_slice(base64(text.as_bytes()).as_bytes());
    out.push(0x07);
    out
}

/// Standard base64 (RFC 4648): the `A-Z a-z 0-9 + /` alphabet, `=` padding, no line breaks. Small enough not to need
/// a crate.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        // Each index is 6 bits, so always in the alphabet.
        let sextet = |shift: u32| char::from(ALPHABET[((n >> shift) & 0x3f) as usize]);
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

/// One task that writes config snapshots in order. A snapshot that arrives while a save is running waits, and a newer
/// one replaces it, so only the newest is written next.
struct ConfigSaver {
    tx: watch::Sender<Option<Box<Config>>>,
    task: JoinHandle<()>,
}

impl ConfigSaver {
    fn spawn(paths: ConfigPaths, events: mpsc::UnboundedSender<Event>) -> Self {
        let (tx, mut rx) = watch::channel::<Option<Box<Config>>>(None);
        let task = tokio::spawn(async move {
            // `changed` reports a value sent just before the sender closed, so the last save isn't lost.
            while rx.changed().await.is_ok() {
                let Some(config) = rx.borrow_and_update().clone() else {
                    continue;
                };
                let paths = paths.clone();
                let result = match tokio::task::spawn_blocking(move || config.save(&paths)).await {
                    Ok(saved) => saved.map_err(|e| e.to_string()),
                    Err(e) => Err(format!("the save task failed: {e}")),
                };
                let _ = events.send(Event::ConfigSaved(result));
            }
        });
        Self { tx, task }
    }

    fn save(&self, config: Box<Config>) {
        // The receiver lives as long as the task, which only ends after `finish` drops the sender.
        let _ = self.tx.send(Some(config));
    }

    async fn finish(self) {
        drop(self.tx);
        if let Err(e) = self.task.await {
            tracing::error!("The config saver failed: {e}");
        }
    }
}

/// One task that writes cache files, one at a time. A write to a path that arrives while an older write to it is
/// still pending replaces that one, so each path gets its newest text next. A write that fails is logged as a warning;
/// the app carries on with its rows.
struct CacheWriter {
    tx: mpsc::UnboundedSender<(PathBuf, String)>,
    task: JoinHandle<()>,
}

impl CacheWriter {
    fn spawn() -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<(PathBuf, String)>();
        let task = tokio::spawn(async move {
            // Pending writes, oldest path first. `insert` keeps a path's place and replaces its text.
            let mut pending: IndexMap<PathBuf, String> = IndexMap::new();
            loop {
                if pending.is_empty() {
                    // `None` once `finish` drops the sender and everything sent before is taken.
                    let Some((path, text)) = rx.recv().await else { break };
                    pending.insert(path, text);
                }
                while let Ok((path, text)) = rx.try_recv() {
                    pending.insert(path, text);
                }
                let Some((path, text)) = pending.shift_remove_index(0) else {
                    continue;
                };
                let written = tokio::task::spawn_blocking({
                    let path = path.clone();
                    move || cache::write(&path, &text)
                })
                .await;
                let error = match written {
                    Ok(Ok(())) => continue,
                    Ok(Err(e)) => e.to_string(),
                    Err(e) => format!("the write task failed: {e}"),
                };
                tracing::warn!("Failed to write cache file '{}': {error}", path.display());
            }
        });
        Self { tx, task }
    }

    fn write(&self, path: PathBuf, text: String) {
        // The receiver lives as long as the task, which only ends after `finish` drops the sender.
        let _ = self.tx.send((path, text));
    }

    async fn finish(self) {
        drop(self.tx);
        if let Err(e) = self.task.await {
            tracing::error!("The cache writer failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use lazy_github_core::github::{FakeGithub, GithubCall};

    use super::*;
    use crate::effect::{GitRequest, GitResponse, GithubRequest, GithubResponse};

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("lazy-github-runtime-test-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn paths(&self) -> ConfigPaths {
            ConfigPaths::in_dir(self.0.join("config"))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn runtime(gh: FakeGithub, dir: &TempDir) -> (Runtime<FakeGithub>, mpsc::UnboundedReceiver<Event>) {
        runtime_with_git(gh, GitCli::new(Some(dir.0.clone())), dir)
    }

    fn runtime_with_git(
        gh: FakeGithub,
        git: GitCli,
        dir: &TempDir,
    ) -> (Runtime<FakeGithub>, mpsc::UnboundedReceiver<Event>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Runtime::new(gh, git, tx, dir.paths()), rx)
    }

    /// Runs `git args` in `dir` with the developer's global and system config out of the way.
    fn git_in(dir: &std::path::Path, home: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("HOME", home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    // Not on a paused clock: the git child is a real process, and auto-advancing time would trip its timeout.
    #[tokio::test(flavor = "current_thread")]
    async fn git_effects_deliver_the_parsed_origin() {
        let dir = TempDir::new();
        let (home, work) = (dir.0.join("home"), dir.0.join("work"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        git_in(&work, &home, &["init", "-q"]);
        git_in(
            &work,
            &home,
            &["remote", "add", "origin", "git@github.com:o/my.repo.git"],
        );
        let git = GitCli::new(Some(work))
            .with_env("HOME", &home)
            .with_env("GIT_CONFIG_NOSYSTEM", "1");
        let (mut runtime, mut rx) = runtime_with_git(FakeGithub::new(), git, &dir);
        let id = RequestId::new(3);
        runtime.execute(Effect::Git {
            id,
            request: GitRequest::OriginRepo,
        });
        match tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Event::Git {
                id: got,
                response: GitResponse::OriginRepo(Some(repo)),
            } => {
                assert_eq!(got, id);
                assert_eq!(repo.to_string(), "o/my.repo");
            }
            other => panic!("unexpected {other:?}"),
        }
        runtime.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_cancelled_git_request_delivers_nothing() {
        let dir = TempDir::new();
        let (mut runtime, mut rx) = runtime(FakeGithub::new(), &dir);
        let id = RequestId::new(2);
        runtime.execute(Effect::Git {
            id,
            request: GitRequest::OriginRepo,
        });
        // Cancelled before its task first runs, so git never starts and nothing can arrive.
        runtime.execute(Effect::Cancel(id));
        run_ready_tasks().await;
        tokio::time::sleep(Duration::from_mins(1)).await;
        assert!(rx.try_recv().is_err());
        runtime.shutdown().await;
    }

    /// Lets every ready task run.
    async fn run_ready_tasks() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    fn timer_ticks(rx: &mut mpsc::UnboundedReceiver<Event>) -> Vec<TimerId> {
        let mut ticks = Vec::new();
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Timer(id) => ticks.push(id),
                other => panic!("unexpected {other:?}"),
            }
        }
        ticks
    }

    /// A writer whose clones share one buffer.
    #[derive(Clone, Default)]
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_bell_writes_bel_to_the_terminal() {
        let dir = TempDir::new();
        let (runtime, _rx) = runtime(FakeGithub::new(), &dir);
        let terminal = SharedBuf::default();
        let mut runtime = runtime.with_terminal(terminal.clone());
        runtime.execute(Effect::Bell);
        runtime.execute(Effect::Bell);
        assert_eq!(*terminal.0.lock().unwrap(), b"\x07\x07");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn set_clipboard_writes_osc_52_to_the_terminal() {
        let dir = TempDir::new();
        let (runtime, _rx) = runtime(FakeGithub::new(), &dir);
        let terminal = SharedBuf::default();
        let mut runtime = runtime.with_terminal(terminal.clone());
        runtime.execute(Effect::SetClipboard("héllo".into()));
        runtime.execute(Effect::SetClipboard(String::new()));
        assert_eq!(*terminal.0.lock().unwrap(), b"\x1b]52;c;aMOpbGxv\x07\x1b]52;c;\x07");
    }

    #[test]
    fn base64_follows_rfc_4648() {
        // The RFC's test vectors.
        for (text, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(text.as_bytes()), encoded, "{text:?}");
        }
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn github_effects_deliver_their_response_with_the_same_id() {
        let dir = TempDir::new();
        let (mut runtime, mut rx) = runtime(FakeGithub::new(), &dir);
        let id = RequestId::new(7);
        runtime.execute(Effect::Github {
            id,
            request: GithubRequest::CheckAuth,
        });
        match rx.recv().await.unwrap() {
            Event::Github {
                id: got,
                response: GithubResponse::CheckAuth(Ok(())),
            } => assert_eq!(got, id),
            other => panic!("unexpected {other:?}"),
        }
        runtime.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_cancelled_request_delivers_nothing() {
        let dir = TempDir::new();
        let gh = FakeGithub::new().with_latency(Duration::from_secs(1));
        let (mut runtime, mut rx) = runtime(gh.clone(), &dir);
        let id = RequestId::new(1);
        runtime.execute(Effect::Github {
            id,
            request: GithubRequest::CheckAuth,
        });
        run_ready_tasks().await;
        assert_eq!(gh.calls(), [GithubCall::CheckAuth]);

        runtime.execute(Effect::Cancel(id));
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(rx.try_recv().is_err());
        runtime.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn saves_queued_during_a_save_collapse_to_the_newest() {
        let dir = TempDir::new();
        let (mut runtime, mut rx) = runtime(FakeGithub::new(), &dir);
        let paths = dir.paths();
        let config = |theme: &str| {
            let mut config = Config::default_in(&paths);
            theme.clone_into(&mut config.appearance.theme);
            Box::new(config)
        };

        // Real theme names, since an unknown one loads back as `textual-dark`.
        runtime.execute(Effect::SaveConfig(config("nord")));
        // The saver takes the first snapshot and starts writing it.
        run_ready_tasks().await;
        runtime.execute(Effect::SaveConfig(config("dracula")));
        runtime.execute(Effect::SaveConfig(config("gruvbox")));
        runtime.shutdown().await;

        let mut saves = 0;
        while let Ok(event) = rx.try_recv() {
            assert!(matches!(event, Event::ConfigSaved(Ok(()))), "{event:?}");
            saves += 1;
        }
        assert_eq!(saves, 2);
        assert_eq!(Config::load(&paths).unwrap().appearance.theme, "gruvbox");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_failed_save_is_reported_as_text() {
        let dir = TempDir::new();
        // A file where the config folder should be makes `create_dir_all` fail.
        std::fs::write(dir.paths().dir, "").unwrap();
        let (mut runtime, mut rx) = runtime(FakeGithub::new(), &dir);
        runtime.execute(Effect::SaveConfig(Box::new(Config::default_in(&dir.paths()))));
        runtime.shutdown().await;
        match rx.try_recv().unwrap() {
            Event::ConfigSaved(Err(message)) => assert!(message.starts_with("failed to write "), "{message}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    async fn cache_read(rx: &mut mpsc::UnboundedReceiver<Event>) -> (RequestId, Result<Option<String>, String>) {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Event::CacheRead { id, result } => (id, result),
            other => panic!("unexpected {other:?}"),
        }
    }

    // Not on a paused clock: the reads run on blocking threads.
    #[tokio::test(flavor = "current_thread")]
    async fn cache_reads_answer_with_the_text_or_none() {
        let dir = TempDir::new();
        let (mut runtime, mut rx) = runtime(FakeGithub::new(), &dir);
        let path = dir.0.join("cache").join("repos.json");
        runtime.execute(Effect::ReadCache {
            id: RequestId::new(4),
            path: path.clone(),
        });
        assert_eq!(cache_read(&mut rx).await, (RequestId::new(4), Ok(None)));

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[]").unwrap();
        runtime.execute(Effect::ReadCache {
            id: RequestId::new(5),
            path: path.clone(),
        });
        assert_eq!(cache_read(&mut rx).await, (RequestId::new(5), Ok(Some("[]".into()))));

        // Text that isn't UTF-8 can't be read; the error comes back as text.
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        runtime.execute(Effect::ReadCache {
            id: RequestId::new(6),
            path,
        });
        let (id, result) = cache_read(&mut rx).await;
        assert_eq!(id, RequestId::new(6));
        assert!(result.is_err(), "{result:?}");
        runtime.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cache_writes_create_the_directory_and_the_newest_wins() {
        let dir = TempDir::new();
        let (mut runtime, _rx) = runtime(FakeGithub::new(), &dir);
        let path = dir.0.join("nested").join(".cache").join("repos.json");
        let other = dir.0.join("nested").join(".cache").join("o_r_issues.json");
        for text in ["[1]", "[2]", "[3]"] {
            runtime.execute(Effect::WriteCache {
                path: path.clone(),
                text: text.into(),
            });
        }
        runtime.execute(Effect::WriteCache {
            path: other.clone(),
            text: "[]".into(),
        });
        runtime.shutdown().await;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[3]");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "[]");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pending_writes_to_one_path_collapse_and_a_failed_write_logs_a_warning() {
        let capture = crate::test_util::LogCapture::start();
        let dir = TempDir::new();
        // A file where the cache folder should be makes every write fail.
        let blocker = dir.0.join("cache");
        std::fs::write(&blocker, "").unwrap();
        let path = blocker.join("repos.json");
        let (mut runtime, _rx) = runtime(FakeGithub::new(), &dir);
        // All three are queued before the writer first runs, so it takes only the newest.
        for text in ["[1]", "[2]", "[3]"] {
            runtime.execute(Effect::WriteCache {
                path: path.clone(),
                text: text.into(),
            });
        }
        runtime.shutdown().await;
        let warnings: Vec<String> = capture
            .text()
            .lines()
            .filter(|line| line.starts_with("WARN"))
            .map(str::to_owned)
            .collect();
        let [warning] = warnings.as_slice() else {
            panic!("{warnings:?}");
        };
        let prefix = format!("WARN Failed to write cache file '{}': ", path.display());
        assert!(warning.starts_with(&prefix), "{warning}");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn intervals_tick_from_one_period_on_until_cancelled() {
        let dir = TempDir::new();
        let (mut runtime, mut rx) = runtime(FakeGithub::new(), &dir);
        let every = Duration::from_secs(10);
        runtime.execute(Effect::StartInterval {
            id: TimerId::Notifications,
            every,
        });
        run_ready_tasks().await;
        // A second start with the same id doesn't add a timer.
        runtime.execute(Effect::StartInterval {
            id: TimerId::Notifications,
            every: Duration::from_secs(1),
        });

        tokio::time::sleep(Duration::from_secs(9)).await;
        assert_eq!(timer_ticks(&mut rx), []);
        tokio::time::sleep(Duration::from_secs(1)).await;
        run_ready_tasks().await;
        assert_eq!(timer_ticks(&mut rx), [TimerId::Notifications]);
        tokio::time::sleep(Duration::from_secs(20)).await;
        run_ready_tasks().await;
        assert_eq!(timer_ticks(&mut rx), [TimerId::Notifications, TimerId::Notifications]);

        runtime.execute(Effect::CancelTimer(TimerId::Notifications));
        tokio::time::sleep(Duration::from_mins(1)).await;
        run_ready_tasks().await;
        assert_eq!(timer_ticks(&mut rx), []);
        runtime.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn timeouts_fire_once_and_a_restart_replaces_them() {
        let dir = TempDir::new();
        let (mut runtime, mut rx) = runtime(FakeGithub::new(), &dir);
        runtime.execute(Effect::StartTimeout {
            id: TimerId::AuthPoll,
            after: Duration::from_secs(5),
        });
        tokio::time::sleep(Duration::from_secs(3)).await;
        // Restarting replaces the running timeout: it now fires at +3+5, not +5.
        runtime.execute(Effect::StartTimeout {
            id: TimerId::AuthPoll,
            after: Duration::from_secs(5),
        });
        tokio::time::sleep(Duration::from_secs(4)).await;
        run_ready_tasks().await;
        assert_eq!(timer_ticks(&mut rx), []);
        tokio::time::sleep(Duration::from_secs(1)).await;
        run_ready_tasks().await;
        assert_eq!(timer_ticks(&mut rx), [TimerId::AuthPoll]);
        tokio::time::sleep(Duration::from_mins(1)).await;
        run_ready_tasks().await;
        assert_eq!(timer_ticks(&mut rx), []);
        runtime.shutdown().await;
    }
}
