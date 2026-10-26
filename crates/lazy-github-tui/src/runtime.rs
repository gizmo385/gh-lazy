//! The effect runner: performs `Effect`s in the background and sends their results back as `Event`s.

use std::collections::HashMap;
use std::io::{self, Write};

use lazy_github_core::config::{Config, ConfigPaths};
use lazy_github_core::github::Github;
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, JoinHandle};
use tokio::time::{Instant, MissedTickBehavior};

use crate::effect::{Effect, RequestId, TimerId};
use crate::event::Event;

/// Runs effects. Every method returns at once; the work happens on spawned tasks.
pub struct Runtime<G: Github> {
    gh: G,
    tx: mpsc::UnboundedSender<Event>,
    tasks: HashMap<RequestId, AbortHandle>,
    timers: HashMap<TimerId, AbortHandle>,
    saver: ConfigSaver,
    /// The terminal, for the bell.
    terminal: Box<dyn Write + Send>,
}

impl<G: Github> Runtime<G> {
    /// A runtime that sends results to `tx` and saves the config to `paths`. Must be called inside a tokio runtime.
    pub fn new(gh: G, tx: mpsc::UnboundedSender<Event>, paths: ConfigPaths) -> Self {
        let saver = ConfigSaver::spawn(paths, tx.clone());
        Self {
            gh,
            tx,
            tasks: HashMap::new(),
            timers: HashMap::new(),
            saver,
            terminal: Box::new(io::stdout()),
        }
    }

    /// Sends the bell to `terminal` instead of stdout.
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
            Effect::Cancel(id) => {
                // Aborting drops the future, and with it the `gh` child (`kill_on_drop`).
                if let Some(task) = self.tasks.remove(&id) {
                    task.abort();
                }
            }
            Effect::SaveConfig(config) => self.saver.save(config),
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

    /// Aborts every request (killing their `gh` children) and timer, then waits for the config saver to write the
    /// newest pending save.
    pub async fn shutdown(self) {
        for task in self.tasks.into_values().chain(self.timers.into_values()) {
            task.abort();
        }
        self.saver.finish().await;
    }
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use lazy_github_core::github::{FakeGithub, GithubCall};

    use super::*;
    use crate::effect::{GithubRequest, GithubResponse};

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
        let (tx, rx) = mpsc::unbounded_channel();
        (Runtime::new(gh, tx, dir.paths()), rx)
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

        runtime.execute(Effect::SaveConfig(config("first")));
        // The saver takes the first snapshot and starts writing it.
        run_ready_tasks().await;
        runtime.execute(Effect::SaveConfig(config("second")));
        runtime.execute(Effect::SaveConfig(config("third")));
        runtime.shutdown().await;

        let mut saves = 0;
        while let Ok(event) = rx.try_recv() {
            assert!(matches!(event, Event::ConfigSaved(Ok(()))), "{event:?}");
            saves += 1;
        }
        assert_eq!(saves, 2);
        assert_eq!(Config::load(&paths).unwrap().appearance.theme, "third");
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
