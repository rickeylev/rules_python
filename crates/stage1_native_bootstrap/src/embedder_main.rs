use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if let Err(err) = stage1_native_bootstrap_lib::run_embedder(args) {
        eprintln!("stage1_native_bootstrap_embedder: {err}");
        process::exit(1);
    }
}
