//! Unified `shrubbery` ("Shell replacements for Bazel build tools in Rust") subcommand dispatcher
//! for `@rules_python` Rust build tools.

use std::ffi::OsString;
use std::process::ExitCode;

/// Canonical subcommand names supported by `shrubbery`.
pub const SUBCOMMANDS: &[&str] = &[
    // keep-sorted start
    "build_data_writer",
    "exe_zip_maker",
    "extract_wheel_files",
    "py_console_script_gen",
    "py_test_main_validator",
    "py_wheel_dist",
    "venv_shebang_rewriter",
    "wheel_record_rewriter",
    "wheelmaker",
    "zip_main_maker",
    "zipper",
    // keep-sorted end
];

fn print_usage(prog_name: &str, to_stderr: bool) {
    let msg = format!(
        "shrubbery — A nice, two-level effect with a little path running down the middle.\n\nUsage: {} <subcommand> [args...]\n\nAvailable subcommands:\n  {}",
        prog_name,
        SUBCOMMANDS.join("\n  ")
    );
    if to_stderr {
        eprintln!("{msg}");
    } else {
        println!("{msg}");
    }
}

/// Identifies the requested subcommand from `args` (including `argv[0]`) and delegates remaining
/// argument parsing and execution to that subcommand's library crate.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    use lexopt::prelude::*;

    let mut parser = lexopt::Parser::from_iter(args);
    let prog_name = parser.bin_name().unwrap_or("shrubbery").to_owned();

    let sub_os = match parser.next() {
        Ok(Some(Short('h') | Long("help"))) => {
            print_usage(&prog_name, false);
            return ExitCode::SUCCESS;
        }
        Ok(Some(Value(sub_os))) => sub_os,
        Ok(Some(other)) => {
            let err = other.unexpected();
            eprintln!("shrubbery: error: {err}");
            print_usage(&prog_name, true);
            return ExitCode::FAILURE;
        }
        Ok(None) => {
            print_usage(&prog_name, true);
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("shrubbery: error: {err}");
            print_usage(&prog_name, true);
            return ExitCode::FAILURE;
        }
    };

    if sub_os == "help" {
        print_usage(&prog_name, false);
        return ExitCode::SUCCESS;
    }

    let Ok(raw_args) = parser.raw_args() else {
        print_usage(&prog_name, true);
        return ExitCode::FAILURE;
    };

    match sub_os.to_str() {
        // keep-sorted start
        Some("build_data_writer") => build_data_writer_lib::run_cli(raw_args),
        Some("exe_zip_maker") => exe_zip_maker_lib::run_cli(raw_args),
        Some("extract_wheel_files") => extract_wheel_files_lib::run_cli(raw_args),
        Some("py_console_script_gen") => py_console_script_gen_lib::run_cli(raw_args),
        Some("py_test_main_validator") => py_test_main_validator_lib::run_cli(raw_args),
        Some("py_wheel_dist") => py_wheel_dist_lib::run_cli(raw_args),
        Some("venv_shebang_rewriter") => venv_shebang_rewriter_lib::run_cli(raw_args),
        Some("wheel_record_rewriter") => wheel_record_rewriter_lib::run_cli(raw_args),
        Some("wheelmaker") => wheelmaker_lib::run_cli(raw_args),
        Some("zip_main_maker") => zip_main_maker_lib::run_cli(raw_args),
        Some("zipper") => zipper_lib::run_cli(raw_args),
        // keep-sorted end
        _ => {
            eprintln!(
                "shrubbery: error: unknown subcommand '{}'",
                sub_os.to_string_lossy()
            );
            print_usage(&prog_name, true);
            ExitCode::FAILURE
        }
    }
}
