//! `fake-browser`: the program `$BROWSER` names in parity runs. See `fake_gh::browser` and `parity/README.md`.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let (_, args) = fake_gh::app::string_args(std::env::args_os());
    let log = std::env::var_os(fake_gh::browser::LOG_ENV).map(PathBuf::from);
    ExitCode::from(fake_gh::browser::run(
        &args,
        log.as_deref(),
        &mut std::io::stderr().lock(),
    ))
}
