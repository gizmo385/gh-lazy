//! The panic hook restores the terminal exactly once. In its own test binary because the hook is process-global.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use lazy_github_tui::tui::{TerminalGuard, install_panic_hook};

#[derive(Clone, Default)]
struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

const RESTORE: &str =
    "\x1b[<1u\x1b[?1004l\x1b[?2004l\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h";

#[test]
fn a_panic_restores_the_terminal_once() {
    let buf = SharedBuf::default();
    let previous_ran = Arc::new(Mutex::new(false));
    let flag = Arc::clone(&previous_ran);
    // Stands in for color-eyre's hook, which must still run after the restore.
    std::panic::set_hook(Box::new(move |_| *flag.lock().unwrap() = true));

    let out = buf.clone();
    let result = std::panic::catch_unwind(move || {
        let guard = TerminalGuard::enter(out.clone(), true).unwrap();
        out.0.lock().unwrap().clear();
        install_panic_hook(guard.restore_fn());
        panic!("boom");
    });
    let _ = std::panic::take_hook();

    assert!(result.is_err());
    assert!(*previous_ran.lock().unwrap());
    // The hook restored first; the guard's drop during unwinding wrote nothing more.
    let written = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert_eq!(written, RESTORE);
}
