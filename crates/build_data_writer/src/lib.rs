//! Library for writing Bazel target build data and workspace status stamping info.

use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Configuration options for writing a `build_data.txt` file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildDataOptions {
    /// Path to the output `build_data.txt` file.
    pub output: PathBuf,
    /// Target label string (e.g. `//pkg:target`).
    pub target: String,
    /// Whether stamping is enabled (`"TRUE"` or `"FALSE"`).
    pub stamped: String,
    /// Optional path to Bazel's `volatile-status.txt` (`ctx.version_file`).
    pub version_file: Option<PathBuf>,
    /// Optional path to Bazel's `stable-status.txt` (`ctx.info_file`).
    pub info_file: Option<PathBuf>,
}

/// Expands `@flagfile` arguments into a flat list of arguments.
pub fn expand_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut expanded = Vec::new();
    for arg in args {
        if let Some(file_path) = arg.strip_prefix('@') {
            let content = fs::read_to_string(file_path)
                .map_err(|e| format!("failed to read flagfile '{}': {}", file_path, e))?;
            for line in content.lines() {
                if !line.is_empty() {
                    expanded.push(line.to_string());
                }
            }
        } else {
            expanded.push(arg.clone());
        }
    }
    Ok(expanded)
}

/// Parses configuration from an environment variable lookup closure and CLI arguments.
///
/// Environment variables (`OUTPUT`, `TARGET`, `STAMPED`, `VERSION_FILE`, `INFO_FILE`)
/// provide initial values, and CLI flags (`--output`, `--target`, `--stamped`,
/// `--version_file`, `--info_file`) override or supplement them.
pub fn parse_env_and_args<F>(args: &[String], get_env: F) -> Result<BuildDataOptions, String>
where
    F: Fn(&str) -> Option<String>,
{
    let expanded = expand_args(args)?;

    let mut output = get_env("OUTPUT").filter(|s| !s.is_empty());
    let mut target = get_env("TARGET").unwrap_or_default();
    let mut stamped = get_env("STAMPED").unwrap_or_default();
    let mut version_file = get_env("VERSION_FILE").filter(|s| !s.is_empty());
    let mut info_file = get_env("INFO_FILE").filter(|s| !s.is_empty());

    let mut i = 0;
    while i < expanded.len() {
        let arg = &expanded[i];
        let (flag, inline_val) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v.to_string())),
            None => (arg.as_str(), None),
        };

        let mut next_val = |flag_name: &str| -> Result<String, String> {
            if let Some(v) = inline_val.clone() {
                Ok(v)
            } else {
                i += 1;
                expanded
                    .get(i)
                    .cloned()
                    .ok_or_else(|| format!("flag '{}' requires a value", flag_name))
            }
        };

        match flag {
            "--output" => {
                let val = next_val("--output")?;
                output = if val.is_empty() { None } else { Some(val) };
            }
            "--target" => {
                target = next_val("--target")?;
            }
            "--stamped" => {
                stamped = next_val("--stamped")?;
            }
            "--version_file" | "--version-file" => {
                let val = next_val(flag)?;
                version_file = if val.is_empty() { None } else { Some(val) };
            }
            "--info_file" | "--info-file" => {
                let val = next_val(flag)?;
                info_file = if val.is_empty() { None } else { Some(val) };
            }
            _ => {
                return Err(format!("unexpected argument: '{}'", arg));
            }
        }
        i += 1;
    }

    let output_path = output.map(PathBuf::from).ok_or_else(|| {
        "missing required OUTPUT environment variable or --output flag".to_string()
    })?;

    Ok(BuildDataOptions {
        output: output_path,
        target,
        stamped,
        version_file: version_file.map(PathBuf::from),
        info_file: info_file.map(PathBuf::from),
    })
}

/// Parses configuration from process environment variables and CLI arguments.
pub fn parse_args(args: &[String]) -> Result<BuildDataOptions, String> {
    parse_env_and_args(args, |key| env::var(key).ok())
}

fn append_status_file<W: Write>(writer: &mut W, path: &Path) -> io::Result<()> {
    let contents = fs::read(path)?;
    if !contents.is_empty() {
        writer.write_all(&contents)?;
        if !contents.ends_with(b"\n") {
            writer.write_all(b"\n")?;
        }
    }
    Ok(())
}

/// Writes the build data file according to `options`.
pub fn write_build_data(options: &BuildDataOptions) -> io::Result<()> {
    if options.output.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output path must not be empty",
        ));
    }

    if let Some(parent) = options.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let file = File::create(&options.output)?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "TARGET {}", options.target)?;
    writeln!(writer, "STAMPED {}", options.stamped)?;

    if let Some(ref version_file) = options.version_file {
        if !version_file.as_os_str().is_empty() {
            append_status_file(&mut writer, version_file)?;
        }
    }

    if let Some(ref info_file) = options.info_file {
        if !info_file.as_os_str().is_empty() {
            append_status_file(&mut writer, info_file)?;
        }
    }

    writer.flush()?;
    Ok(())
}

/// Parses process environment variables and CLI arguments, then writes the build data file.
pub fn run(args: &[String]) -> Result<(), String> {
    let options = parse_args(args)?;
    write_build_data(&options).map_err(|e| {
        format!(
            "failed to write build data to '{}': {}",
            options.output.display(),
            e
        )
    })
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), writes build data, and returns
/// the process exit code.
pub fn run_cli<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let str_args: Vec<String> = args
        .into_iter()
        .map(|a| a.into().to_string_lossy().into_owned())
        .collect();
    if let Err(err) = run(&str_args) {
        eprintln!("build_data_writer: error: {}", err);
        return 1;
    }
    0
}
