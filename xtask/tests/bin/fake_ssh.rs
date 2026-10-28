//! fake-ssh built as an xtask example, so `tests/local_remote.rs` has the binary without building the fake-gh crate's
//! bins: the same `main` as `crates/fake-gh/src/bin/fake-ssh.rs`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let (_, args) = fake_gh::app::string_args(std::env::args_os());
    let env = fake_gh::ssh::Env::from_process();
    ExitCode::from(fake_gh::ssh::run(&args, &env, &mut std::io::stderr().lock()))
}
