//! Library for writing Bazel target build data and workspace status stamping info.

use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Configuration options for writing a `build_data.txt` file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildDataOptions {
    /// Path to the output `build_data.txt` file.
    pub output: PathBuf,
    /// Target label string (e.g. `//pkg:target`).
    pub target: String,
    /// Whether stamping is enabled.
    pub stamped: bool,
    /// Optional path to Bazel's `volatile-status.txt` (`ctx.version_file`).
    pub version_file: Option<PathBuf>,
    /// Optional path to Bazel's `stable-status.txt` (`ctx.info_file`).
    pub info_file: Option<PathBuf>,
}

/// Expands `@flagfile` arguments into a flat list of `OsString` arguments, skipping blank lines
/// inside flagfiles and rejecting bare `"@"`.
pub fn expand_args<I, T>(args: I) -> Result<Vec<OsString>, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let raw_args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if raw_args.iter().any(|arg| arg == "@") {
        return Err("flagfile path after '@' must not be empty".to_string());
    }
    argfile::expand_args_from(
        raw_args.into_iter(),
        |content, prefix| {
            content
                .lines()
                .filter(|line| !line.is_empty())
                .map(|line| argfile::Argument::parse_ref(line, prefix))
                .collect()
        },
        argfile::PREFIX,
    )
    .map_err(|e| format!("failed to read flagfile: {e}"))
}

fn parse_stamped(val: &str) -> Result<bool, String> {
    match val {
        "TRUE" | "true" | "1" => Ok(true),
        "FALSE" | "false" | "0" | "" => Ok(false),
        other => Err(format!(
            "invalid boolean value for STAMPED/--stamped: '{other}'"
        )),
    }
}

fn format_lexopt_error(err: lexopt::Error) -> String {
    match err {
        lexopt::Error::MissingValue { option: Some(opt) } => {
            format!("flag '{opt}' requires a value")
        }
        lexopt::Error::UnexpectedOption(opt) => format!("unexpected argument: '{opt}'"),
        lexopt::Error::UnexpectedArgument(val) => {
            format!("unexpected argument: '{}'", val.to_string_lossy())
        }
        other => other.to_string(),
    }
}

/// Parses configuration from an environment variable lookup closure and CLI arguments.
///
/// Environment variables (`OUTPUT`, `TARGET`, `STAMPED`, `VERSION_FILE`, `INFO_FILE`)
/// provide initial values, and CLI flags (`--output`, `--target`, `--stamped`,
/// `--version_file`, `--info_file`) override or supplement them.
pub fn parse_env_and_args<I, T, F>(args: I, get_env: F) -> Result<BuildDataOptions, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
    F: Fn(&str) -> Option<OsString>,
{
    use lexopt::prelude::*;

    let expanded = expand_args(args)?;

    let mut output = get_env("OUTPUT")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let mut target = match get_env("TARGET") {
        Some(val) => val
            .into_string()
            .map_err(|raw| format!("TARGET environment variable is invalid UTF-8: {raw:?}"))?,
        None => String::new(),
    };
    let mut stamped = match get_env("STAMPED") {
        Some(val) => {
            let raw = val
                .into_string()
                .map_err(|raw| format!("STAMPED environment variable is invalid UTF-8: {raw:?}"))?;
            parse_stamped(&raw)?
        }
        None => false,
    };
    let mut version_file = get_env("VERSION_FILE")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let mut info_file = get_env("INFO_FILE")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);

    let mut parser = lexopt::Parser::from_args(expanded);
    while let Some(arg) = parser.next().map_err(format_lexopt_error)? {
        match arg {
            Long("output") => {
                let val = parser.value().map_err(format_lexopt_error)?;
                output = (!val.is_empty()).then(|| PathBuf::from(val));
            }
            Long("target") => {
                target = parser
                    .value()
                    .map_err(format_lexopt_error)?
                    .string()
                    .map_err(format_lexopt_error)?;
            }
            Long("stamped") => {
                let raw = parser
                    .value()
                    .map_err(format_lexopt_error)?
                    .string()
                    .map_err(format_lexopt_error)?;
                stamped = parse_stamped(&raw)?;
            }
            Long("version_file" | "version-file") => {
                let val = parser.value().map_err(format_lexopt_error)?;
                version_file = (!val.is_empty()).then(|| PathBuf::from(val));
            }
            Long("info_file" | "info-file") => {
                let val = parser.value().map_err(format_lexopt_error)?;
                info_file = (!val.is_empty()).then(|| PathBuf::from(val));
            }
            _ => return Err(format_lexopt_error(arg.unexpected())),
        }
    }

    let output = output
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| {
            "missing required OUTPUT environment variable or --output flag".to_string()
        })?;

    Ok(BuildDataOptions {
        output,
        target,
        stamped,
        version_file,
        info_file,
    })
}

/// Parses configuration from process environment variables and CLI arguments.
pub fn parse_args<I, T>(args: I) -> Result<BuildDataOptions, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    parse_env_and_args(args, |key| env::var_os(key))
}

/// Parses process environment variables and CLI arguments, then writes the build data file.
pub fn run<I, T>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = parse_args(args)?;
    write_build_data(&options).map_err(|e| {
        format!(
            "failed to write build data to '{}': {e}",
            options.output.display(),
        )
    })
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), writes build data, and returns
/// the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    if let Err(err) = run(args) {
        eprintln!("build_data_writer: error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Writes the build data file according to `options`.
pub fn write_build_data(options: &BuildDataOptions) -> io::Result<()> {
    if let Some(parent) = options.output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(&options.output)?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "TARGET {}", options.target)?;
    writeln!(
        writer,
        "STAMPED {}",
        if options.stamped { "TRUE" } else { "FALSE" }
    )?;

    if let Some(version_file) = &options.version_file
        && !version_file.as_os_str().is_empty()
    {
        append_status_file(&mut writer, version_file)?;
    }

    if let Some(info_file) = &options.info_file
        && !info_file.as_os_str().is_empty()
    {
        append_status_file(&mut writer, info_file)?;
    }

    writer.flush()?;
    Ok(())
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
