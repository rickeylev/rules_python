//! Unified `rupyru-box` ("rules python rust box") subcommand dispatcher for `@rules_python`
//! Rust build tools.

use std::ffi::OsString;

/// Canonical subcommand names supported by `rupyru-box`.
pub const SUBCOMMANDS: &[&str] = &[
    // keep-sorted start
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
        "Usage: {} <subcommand> [args...]\n\nAvailable subcommands:\n  {}",
        prog_name,
        SUBCOMMANDS.join("\n  ")
    );
    if to_stderr {
        eprintln!("{}", msg);
    } else {
        println!("{}", msg);
    }
}

/// Identifies the requested subcommand from `args` (including `argv[0]`) and delegates remaining
/// argument parsing and execution to that subcommand's library crate.
pub fn run_cli<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut iter = args.into_iter().map(Into::into);
    let prog_name = iter
        .next()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rupyru-box".to_string());

    let Some(sub_os) = iter.next() else {
        print_usage(&prog_name, true);
        return 1;
    };
    let sub_raw = sub_os.to_string_lossy();
    if sub_raw == "-h" || sub_raw == "--help" || sub_raw == "help" {
        print_usage(&prog_name, false);
        return 0;
    }

    let normalized = if sub_raw.starts_with('-') {
        sub_raw.as_ref().to_owned()
    } else {
        sub_raw.replace('-', "_")
    };

    match normalized.as_str() {
        // keep-sorted start
        "exe_zip_maker" => exe_zip_maker_lib::run_cli(iter),
        "extract_wheel_files" => extract_wheel_files_lib::run_cli(iter),
        "py_console_script_gen" => py_console_script_gen_lib::run_cli(iter),
        "py_test_main_validator" => py_test_main_validator_lib::run_cli(iter),
        "py_wheel_dist" => py_wheel_dist_lib::run_cli(iter),
        "venv_shebang_rewriter" => venv_shebang_rewriter_lib::run_cli(iter),
        "wheel_record_rewriter" => wheel_record_rewriter_lib::run_cli(iter),
        "wheelmaker" => wheelmaker_lib::run_cli(iter),
        "zip_main_maker" => zip_main_maker_lib::run_cli(iter),
        "zipper" => zipper_lib::run_cli(iter),
        // keep-sorted end
        _ => {
            eprintln!("rupyru-box: error: unknown subcommand '{}'", sub_raw);
            print_usage(&prog_name, true);
            1
        }
    }
}
