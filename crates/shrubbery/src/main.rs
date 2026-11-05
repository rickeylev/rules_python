use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    shrubbery_lib::run_cli(env::args_os())
}
