//! fake-gh, built as an example of this package so `tests/exit_paths.rs` can find it: a test can only name its own
//! package's binaries, and `cargo test` builds examples next to the test binaries.

use std::process::ExitCode;

fn main() -> ExitCode {
    let (program, args) = fake_gh::app::string_args(std::env::args_os());
    let env = fake_gh::app::Env::from_process();
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut io = fake_gh::app::Stdio {
        stdin: &mut stdin,
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    ExitCode::from(fake_gh::app::run(&program, &args, &env, &mut io))
}
