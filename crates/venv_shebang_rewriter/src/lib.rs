//! Rewrites a `console_script` wrapper's shebang into a batch/shell-Python polyglot.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Shell-Python polyglot header for Unix-like platforms.
pub const UNIX_HEADER: &[u8] =
    b"#!/bin/sh\n'''exec' \"$(dirname \"$0\")/python3\" \"$0\" \"$@\"\n' '''\n";

/// Batch-Python polyglot header for Windows (`python.exe`).
pub const WINDOWS_PYTHON_HEADER: &[u8] =
    b"@setlocal enabledelayedexpansion & \"%~dp0python.exe\" -x \"%~f0\" %* & exit /b !ERRORLEVEL!\r\n";

/// Batch-Python polyglot header for Windows GUI scripts (`pythonw.exe`).
pub const WINDOWS_PYTHONW_HEADER: &[u8] =
    b"@setlocal enabledelayedexpansion & \"%~dp0pythonw.exe\" -x \"%~f0\" %* & exit /b !ERRORLEVEL!\r\n";

/// Parsed command-line configuration for `venv_shebang_rewriter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub in_path: PathBuf,
    pub out_path: PathBuf,
    pub target_os: String,
}

/// Parses the positional arguments (`<in_path> <out_path> <target_os>`), excluding `argv[0]`.
///
/// Takes the first 3 positional arguments (`argv[1..4]`), matching `venv_shebang_rewriter.py`.
pub fn parse_args<I, S>(args: I) -> Result<Config, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let collected: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if collected.len() < 3 {
        return Err("Usage: venv_shebang_rewriter <in_path> <out_path> <target_os>".to_string());
    }

    let in_path = PathBuf::from(&collected[0]);
    let out_path = PathBuf::from(&collected[1]);
    let target_os = collected[2].to_string_lossy().into_owned();

    Ok(Config {
        in_path,
        out_path,
        target_os,
    })
}

/// Splits `input` at the first `b'\n'` byte into `(first_line, rest)`.
///
/// `first_line` includes the trailing `b'\n'` if one is present, or the entire slice
/// if `input` contains no newline. `rest` contains all bytes after the first `b'\n'`.
pub fn split_first_line(input: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = input.iter().position(|&b| b == b'\n') {
        (&input[..=pos], &input[pos + 1..])
    } else {
        (input, b"")
    }
}

/// Rewrites the first line of `input` into a batch or shell-Python polyglot header,
/// preserving the remaining bytes verbatim.
pub fn rewrite_shebang_bytes(input: &[u8], target_os: &str) -> Vec<u8> {
    let (first_line, rest) = split_first_line(input);
    let header = if target_os == "windows" {
        if first_line.starts_with(b"#!pythonw") {
            WINDOWS_PYTHONW_HEADER
        } else {
            WINDOWS_PYTHON_HEADER
        }
    } else {
        UNIX_HEADER
    };

    let mut output = Vec::with_capacity(header.len() + rest.len());
    output.extend_from_slice(header);
    output.extend_from_slice(rest);
    output
}

/// Reads `in_path`, rewrites the shebang for `target_os`, writes the result to `out_path`,
/// and sets executable permissions (`mode | 0o111`) on Unix.
pub fn rewrite_shebang(in_path: &Path, out_path: &Path, target_os: &str) -> io::Result<()> {
    let input = fs::read(in_path)?;
    let output = rewrite_shebang_bytes(&input, target_os);

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    fs::write(out_path, &output)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(out_path)?;
        let mut perms = metadata.permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(out_path, perms)?;
    }

    Ok(())
}

/// Parses CLI arguments (excluding `argv[0]`) and executes the shebang rewrite.
pub fn run<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let config = parse_args(args)?;
    rewrite_shebang(&config.in_path, &config.out_path, &config.target_os).map_err(|e| e.to_string())
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), rewrites the shebang, and
/// returns the process exit code.
pub fn run_cli<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    if let Err(err) = run(args) {
        eprintln!("venv_shebang_rewriter: error: {}", err);
        return 1;
    }
    0
}
