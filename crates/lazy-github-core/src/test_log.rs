//! Support for tests that capture what code logs with a thread-scoped subscriber (`with_default`, `set_default`).
//!
//! `tracing` caches each callsite's interest process-wide. While at most one dispatcher is registered, it computes a
//! callsite's interest from the default of whichever thread hits the callsite first, ignoring dispatchers scoped to
//! other threads. So when a test thread has a capturing subscriber in scope and another test thread, with none, hits
//! a shared `warn!` first, that callsite is cached as never interested and the capture misses the event (#274).
//! [`keep_callsites_interested`] closes that gap by installing a global subscriber that is interested in everything
//! and records nothing.

use std::sync::Once;

use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};

/// Installs, once per process, a global default subscriber that answers "sometimes" for every callsite, so no
/// callsite is cached as never interested and each event asks the current thread's subscriber instead. Call it
/// before installing a capturing subscriber. A test binary that sets its own global default (such as one testing
/// [`crate::logging::init`]) keeps it.
pub fn keep_callsites_interested() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = tracing::subscriber::set_global_default(Interested);
    });
}

/// Interested in every callsite at every level, enabled for none of them.
struct Interested;

impl Subscriber for Interested {
    fn register_callsite(&self, _: &'static Metadata<'static>) -> Interest {
        Interest::sometimes()
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }

    fn enabled(&self, _: &Metadata<'_>) -> bool {
        false
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        // Never called: `enabled` is false for every span. Ids must be non-zero.
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A `warn!` callsite only this test hits.
    fn warn_once() {
        tracing::warn!("from the shared callsite");
    }

    /// Counts the events it sees.
    struct Count(Arc<Mutex<usize>>);

    impl Subscriber for Count {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }
        fn record(&self, _: &Id, _: &Record<'_>) {}
        fn record_follows_from(&self, _: &Id, _: &Id) {}
        fn event(&self, _: &Event<'_>) {
            *self.0.lock().unwrap() += 1;
        }
        fn enter(&self, _: &Id) {}
        fn exit(&self, _: &Id) {}
    }

    /// The race in #274, made deterministic: another thread, with no subscriber, registers the callsite while this
    /// thread's capture is in scope. Without the global subscriber, and with no other test's dispatcher alive, the
    /// callsite would be cached as never interested and the count would be 0.
    #[test]
    fn a_capture_sees_a_callsite_another_thread_registered_first() {
        keep_callsites_interested();
        let seen = Arc::new(Mutex::new(0));
        tracing::subscriber::with_default(Count(Arc::clone(&seen)), || {
            std::thread::spawn(warn_once).join().unwrap();
            warn_once();
        });
        assert_eq!(*seen.lock().unwrap(), 1);
    }
}
