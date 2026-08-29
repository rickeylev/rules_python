use std::env;
use uv_lock_lib::{print_help, run, Config};

fn main() {
    let config = match Config::load() {
        Ok(c) => c,
        Err(e) if e == "HELP" => {
            print_help();
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            print_help();
            std::process::exit(1);
        }
    };

    let in_workspace = env::var_os("BUILD_WORKSPACE_DIRECTORY").is_some();
    match run(&config, in_workspace) {
        Ok(exit_code) => std::process::exit(exit_code),
        Err(e) => {
            eprintln!("uv_lock error: {}", e);
            std::process::exit(1);
        }
    }
}
