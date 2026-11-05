use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use shrubbery_lib::{SUBCOMMANDS, run_cli};

fn temp_test_dir(prefix: &str) -> PathBuf {
    let base = env::var_os("TEST_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let dir = base.join(format!("shrubbery_{}_{}", prefix, std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_subcommands_registry_sorted_and_complete() {
    assert_eq!(SUBCOMMANDS.len(), 11);
    let mut sorted = SUBCOMMANDS.to_vec();
    sorted.sort_unstable();
    assert_eq!(SUBCOMMANDS, sorted.as_slice());
}

#[test]
fn test_help_flags_exit_zero() {
    assert_eq!(run_cli(["shrubbery", "--help"]), ExitCode::SUCCESS);
    assert_eq!(run_cli(["shrubbery", "-h"]), ExitCode::SUCCESS);
    assert_eq!(run_cli(["shrubbery", "help"]), ExitCode::SUCCESS);
}

#[test]
fn test_missing_or_unknown_subcommand_exits_one() {
    assert_eq!(run_cli(["shrubbery"]), ExitCode::FAILURE);
    assert_eq!(run_cli(["shrubbery", "does_not_exist"]), ExitCode::FAILURE);
    assert_eq!(run_cli(["shrubbery", "--unknown-flag"]), ExitCode::FAILURE);
}

#[test]
fn test_subcommand_arg_parsing_delegation() {
    assert_eq!(run_cli(["shrubbery", "exe_zip_maker"]), ExitCode::FAILURE);
    assert_eq!(
        run_cli(["shrubbery", "exe_zip_maker", "--help"]),
        ExitCode::FAILURE
    );
    assert_eq!(
        run_cli(["shrubbery", "venv_shebang_rewriter", "only_one"]),
        ExitCode::FAILURE
    );
    assert_eq!(
        run_cli(["shrubbery", "py_test_main_validator", "--unknown-flag"]),
        ExitCode::from(2)
    );
}

#[test]
fn test_subcommand_forwarding_and_hyphen_rejection() {
    let dir = temp_test_dir("dispatch");
    let preamble = dir.join("preamble.sh");
    let zip_in = dir.join("in.zip");
    let out_underscore = dir.join("out_underscore.pyz");
    let out_hyphen = dir.join("out_hyphen.pyz");

    fs::write(&preamble, b"#!/bin/sh\n").unwrap();
    fs::write(
        &zip_in,
        b"PK\x05\x06\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
    )
    .unwrap();

    assert_eq!(
        run_cli([
            "shrubbery",
            "exe_zip_maker",
            preamble.to_str().unwrap(),
            zip_in.to_str().unwrap(),
            out_underscore.to_str().unwrap(),
        ]),
        ExitCode::SUCCESS
    );
    assert!(out_underscore.exists());

    assert_eq!(
        run_cli([
            "shrubbery",
            "exe-zip-maker",
            preamble.to_str().unwrap(),
            zip_in.to_str().unwrap(),
            out_hyphen.to_str().unwrap(),
        ]),
        ExitCode::FAILURE
    );
    assert!(!out_hyphen.exists());
}

#[test]
fn test_cli_binary_subprocess() {
    let Some(bin_rel) = env::var_os("SHRUBBERY_BIN") else {
        return;
    };
    let bin_path = PathBuf::from(&bin_rel);
    let resolved = if bin_path.exists() {
        bin_path
    } else if let Ok(srcdir) = env::var("TEST_SRCDIR") {
        let ws = env::var("TEST_WORKSPACE").unwrap_or_else(|_| "_main".to_string());
        PathBuf::from(srcdir).join(ws).join(&bin_rel)
    } else {
        return;
    };
    if !resolved.exists() {
        return;
    }

    let help_out = Command::new(&resolved).arg("--help").output().unwrap();
    assert_eq!(help_out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&help_out.stdout);
    assert!(
        stdout.contains("A nice, two-level effect"),
        "missing Monty Python header in help: {stdout}"
    );
    for sub in SUBCOMMANDS {
        assert!(stdout.contains(sub), "missing {sub} in help: {stdout}");
    }

    let bad_out = Command::new(&resolved)
        .arg("not_a_subcommand")
        .output()
        .unwrap();
    assert_eq!(bad_out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&bad_out.stderr).contains("unknown subcommand"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&bad_out.stderr)
    );
}
