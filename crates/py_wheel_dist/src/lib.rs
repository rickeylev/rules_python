//! Library for generating the output directory for `py_wheel_dist`.
//!
//! Usage:
//!   `py_wheel_dist --wheel <path> --name_file <path> --output <path>`
//!
//! Reads the canonical wheel filename from `--name_file` (UTF-8, whitespace-trimmed),
//! creates the `--output` directory, and copies `--wheel` to `<output>/<wheel_name>`.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Parsed command-line options for `py_wheel_dist`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Path to the input `.whl` file.
    pub wheel: PathBuf,
    /// Path to the file containing the canonical wheel filename.
    pub name_file: PathBuf,
    /// Path to the output directory where the wheel should be copied.
    pub output: PathBuf,
}

/// Expands `@flagfile` arguments into individual argument strings.
fn expand_flagfiles<I, S>(args: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut expanded = Vec::new();
    for arg_item in args {
        let arg = arg_item.as_ref();
        if let Some(flagfile_path) = arg.strip_prefix('@') {
            if flagfile_path.is_empty() {
                return Err("empty @flagfile path".to_string());
            }
            let content = fs::read_to_string(flagfile_path)
                .map_err(|e| format!("failed to read flagfile '{}': {}", flagfile_path, e))?;
            for line in content.lines() {
                if !line.is_empty() {
                    expanded.push(line.to_string());
                }
            }
        } else {
            expanded.push(arg.to_string());
        }
    }
    Ok(expanded)
}

/// Parses command-line arguments (excluding `argv[0]`) into [`Options`].
///
/// Supports `--flag value`, `--flag=value`, and `@flagfile` argument expansion.
pub fn parse_args<I, S>(args: I) -> Result<Options, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let expanded = expand_flagfiles(args)?;
    let mut wheel: Option<PathBuf> = None;
    let mut name_file: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;

    let mut iter = expanded.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "-h" || arg == "--help" {
            return Err(
                "Usage: py_wheel_dist --wheel <path> --name_file <path> --output <path>"
                    .to_string(),
            );
        } else if let Some(val) = arg.strip_prefix("--wheel=") {
            if val.is_empty() {
                return Err("empty value for --wheel".to_string());
            }
            wheel = Some(PathBuf::from(val));
        } else if arg == "--wheel" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --wheel".to_string())?;
            if val.is_empty() {
                return Err("empty value for --wheel".to_string());
            }
            wheel = Some(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--name_file=") {
            if val.is_empty() {
                return Err("empty value for --name_file".to_string());
            }
            name_file = Some(PathBuf::from(val));
        } else if arg == "--name_file" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --name_file".to_string())?;
            if val.is_empty() {
                return Err("empty value for --name_file".to_string());
            }
            name_file = Some(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--output=") {
            if val.is_empty() {
                return Err("empty value for --output".to_string());
            }
            output = Some(PathBuf::from(val));
        } else if arg == "--output" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --output".to_string())?;
            if val.is_empty() {
                return Err("empty value for --output".to_string());
            }
            output = Some(PathBuf::from(val));
        } else {
            return Err(format!("unexpected argument: {}", arg));
        }
    }

    let wheel = wheel.ok_or_else(|| "missing required argument: --wheel".to_string())?;
    let name_file =
        name_file.ok_or_else(|| "missing required argument: --name_file".to_string())?;
    let output = output.ok_or_else(|| "missing required argument: --output".to_string())?;

    Ok(Options {
        wheel,
        name_file,
        output,
    })
}

/// Validates and trims the wheel filename read from `--name_file`.
///
/// Ensures the trimmed name is non-empty and a valid single file basename without
/// path separators (`/`, `\`) or parent directory traversal (`..`).
pub fn validate_wheel_name(raw_name: &str) -> io::Result<&str> {
    let trimmed = raw_name.trim();
    if trimmed.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "wheel name in --name_file is empty",
        ));
    }
    if trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.contains('\0')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid wheel filename in --name_file: '{}'", trimmed),
        ));
    }
    Ok(trimmed)
}

/// Creates the output directory and copies the wheel file to `<output>/<wheel_name>`.
pub fn create_wheel_dist(
    wheel_path: &Path,
    name_file_path: &Path,
    output_dir: &Path,
) -> io::Result<PathBuf> {
    let raw_name = fs::read_to_string(name_file_path)?;
    let wheel_name = validate_wheel_name(&raw_name)?;

    fs::create_dir_all(output_dir)?;

    let destination = output_dir.join(wheel_name);
    fs::copy(wheel_path, &destination)?;
    Ok(destination)
}

/// Executes `py_wheel_dist` with the parsed [`Options`].
pub fn run(options: &Options) -> io::Result<PathBuf> {
    create_wheel_dist(&options.wheel, &options.name_file, &options.output)
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), populates the wheel dist
/// directory, and returns the process exit code.
pub fn run_cli<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let str_args: Vec<String> = args
        .into_iter()
        .map(|a| a.into().to_string_lossy().into_owned())
        .collect();
    let options = match parse_args(&str_args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("py_wheel_dist: error: {}", msg);
            return 1;
        }
    };
    if let Err(err) = run(&options) {
        eprintln!("py_wheel_dist: error: {}", err);
        return 1;
    }
    0
}
