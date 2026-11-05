//! Rewrites a `console_script` wrapper's shebang into a batch/shell-Python polyglot.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

/// Shell-Python polyglot header for Unix-like platforms.
pub const UNIX_HEADER: &[u8] =
    b"#!/bin/sh\n'''exec' \"$(dirname \"$0\")/python3\" \"$0\" \"$@\"\n' '''\n";

/// Batch-Python polyglot header for Windows (`python.exe`).
pub const WINDOWS_PYTHON_HEADER: &[u8] =
    b"@setlocal enabledelayedexpansion & \"%~dp0python.exe\" -x \"%~f0\" %* & exit /b !ERRORLEVEL!\r\n";

/// Batch-Python polyglot header for Windows GUI scripts (`pythonw.exe`).
pub const WINDOWS_PYTHONW_HEADER: &[u8] =
    b"@setlocal enabledelayedexpansion & \"%~dp0pythonw.exe\" -x \"%~f0\" %* & exit /b !ERRORLEVEL!\r\n";

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
pub fn rewrite_shebang_bytes(input: &[u8], target_os: TargetOs) -> Vec<u8> {
    let (first_line, rest) = split_first_line(input);
    let header = match target_os {
        TargetOs::Windows if first_line.starts_with(b"#!pythonw") => WINDOWS_PYTHONW_HEADER,
        TargetOs::Windows => WINDOWS_PYTHON_HEADER,
        TargetOs::Unix => UNIX_HEADER,
    };

    let mut output = Vec::with_capacity(header.len() + rest.len());
    output.extend_from_slice(header);
    output.extend_from_slice(rest);
    output
}

/// Reads `in_path`, rewrites the shebang for `target_os`, writes the result to `out_path`,
/// and sets executable permissions (`mode | 0o111`) on Unix.
pub fn rewrite_shebang(in_path: &Path, out_path: &Path, target_os: TargetOs) -> io::Result<()> {
    let input = fs::read(in_path)?;
    let output = rewrite_shebang_bytes(&input, target_os);

    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
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

/// Target operating system family for generated shebang headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    Unix,
    Windows,
}

impl FromStr for TargetOs {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unix" => Ok(Self::Unix),
            "windows" => Ok(Self::Windows),
            other => Err(format!(
                "unsupported target_os '{other}': expected 'unix' or 'windows'"
            )),
        }
    }
}

/// Parsed command-line options for `venv_shebang_rewriter`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub in_path: PathBuf,
    pub out_path: PathBuf,
    pub target_os: TargetOs,
}

/// Parses the positional arguments (`<in_path> <out_path> <target_os>`), excluding `argv[0]`.
///
/// Takes the first 3 positional arguments (`argv[1..4]`), matching `venv_shebang_rewriter.py`.
pub fn parse_args<I, T>(args: I) -> Result<Options, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    use lexopt::prelude::*;

    const USAGE: &str = "Usage: venv_shebang_rewriter <in_path> <out_path> <target_os>";
    let mut in_path = None;
    let mut out_path = None;
    let mut target_os = None;

    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next().map_err(|e| format!("{e}\n{USAGE}"))? {
        match arg {
            Short('h') | Long("help") => return Err(USAGE.to_string()),
            Value(val) if in_path.is_none() => {
                if val.is_empty() {
                    return Err(format!("in_path must not be empty\n{USAGE}"));
                }
                in_path = Some(PathBuf::from(val));
            }
            Value(val) if out_path.is_none() => {
                if val.is_empty() {
                    return Err(format!("out_path must not be empty\n{USAGE}"));
                }
                out_path = Some(PathBuf::from(val));
            }
            Value(val) if target_os.is_none() => {
                target_os = Some(val.parse::<TargetOs>().map_err(|e| e.to_string())?);
            }
            Value(_) => {}
            _ => return Err(format!("{}\n{USAGE}", arg.unexpected())),
        }
    }

    let (Some(in_path), Some(out_path), Some(target_os)) = (in_path, out_path, target_os) else {
        return Err(USAGE.to_string());
    };

    Ok(Options {
        in_path,
        out_path,
        target_os,
    })
}

/// Parses CLI arguments (excluding `argv[0]`) and executes the shebang rewrite.
pub fn run<I, T>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = parse_args(args)?;
    rewrite_shebang(&options.in_path, &options.out_path, options.target_os)
        .map_err(|e| e.to_string())
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), rewrites the shebang, and
/// returns the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    if let Err(err) = run(args) {
        eprintln!("venv_shebang_rewriter: error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
