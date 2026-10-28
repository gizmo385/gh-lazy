//! `fake-ssh`: the SSH program git runs in parity scenarios with a local remote. See `fake_gh::ssh` and
//! `parity/README.md`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let (_, args) = fake_gh::app::string_args(std::env::args_os());
    let env = fake_gh::ssh::Env::from_process();
    ExitCode::from(fake_gh::ssh::run(&args, &env, &mut std::io::stderr().lock()))
}
