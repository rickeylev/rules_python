//! Rewrites a wheel's `RECORD` file to reflect its final installed layout.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

/// Target operating system layout for PEP 376 wheel `RECORD` path rewriting.
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
                "unsupported target_os '{other}'; expected 'unix' or 'windows'"
            )),
        }
    }
}

/// Parsed command-line arguments for rewriting a wheel `RECORD` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteOptions {
    pub in_path: PathBuf,
    pub out_path: PathBuf,
    pub target_os: TargetOs,
    pub data_dir_basename: String,
    pub rewritten_scripts: HashSet<String>,
}

/// Alias for [`RewriteOptions`].
pub type Options = RewriteOptions;

/// Parses positional command-line arguments (excluding `argv[0]`).
///
/// Expected positional arguments:
/// `<in_path> <out_path> <target_os> <data_dir_basename> [rewritten_scripts...]`
pub fn parse_args<I, T>(args: I) -> Result<RewriteOptions, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    use lexopt::prelude::*;

    let usage = || {
        "Usage: wheel_record_rewriter <in_path> <out_path> <target_os> <data_dir_basename> [rewritten_scripts...]"
            .to_string()
    };

    let mut parser = lexopt::Parser::from_args(args);
    let mut positionals: Vec<OsString> = Vec::new();
    while let Some(arg) = parser.next().map_err(|_| usage())? {
        match arg {
            Value(val) => positionals.push(val),
            Short(_) | Long(_) => return Err(usage()),
        }
    }

    if positionals.len() < 4 {
        return Err(usage());
    }

    let mut iter = positionals.into_iter();
    let in_path = PathBuf::from(iter.next().unwrap());
    let out_path = PathBuf::from(iter.next().unwrap());
    let target_os_str = iter
        .next()
        .unwrap()
        .into_string()
        .map_err(|raw| format!("invalid UTF-8 in target_os: {raw:?}"))?;
    let target_os = target_os_str.parse::<TargetOs>()?;
    let data_dir_basename = iter
        .next()
        .unwrap()
        .into_string()
        .map_err(|raw| format!("invalid UTF-8 in data_dir_basename: {raw:?}"))?;

    let mut rewritten_scripts = HashSet::with_capacity(iter.len());
    for raw_script in iter {
        let script = raw_script
            .into_string()
            .map_err(|raw| format!("invalid UTF-8 in rewritten script name: {raw:?}"))?;
        rewritten_scripts.insert(script);
    }

    Ok(RewriteOptions {
        in_path,
        out_path,
        target_os,
        data_dir_basename,
        rewritten_scripts,
    })
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), rewrites the wheel `RECORD`
/// file, and returns the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = match parse_args(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("wheel_record_rewriter: error: {msg}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = rewrite_record(&options) {
        eprintln!(
            "wheel_record_rewriter: error rewriting {} to {}: {err}",
            options.in_path.display(),
            options.out_path.display(),
        );
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Replacement prefixes corresponding to PEP 376 installation paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplacementPrefixes {
    data_repl: &'static str,
    headers_repl: &'static str,
    platlib_repl: &'static str,
    purelib_repl: &'static str,
    scripts_repl: &'static str,
}

impl ReplacementPrefixes {
    const fn for_target_os(target_os: TargetOs) -> Self {
        match target_os {
            TargetOs::Windows => Self {
                data_repl: "../../",
                headers_repl: "../../Include/",
                platlib_repl: "",
                purelib_repl: "",
                scripts_repl: "../../Scripts/",
            },
            TargetOs::Unix => Self {
                data_repl: "../../../",
                headers_repl: "../../../include/",
                platlib_repl: "",
                purelib_repl: "",
                scripts_repl: "../../../bin/",
            },
        }
    }
}

fn rewrite_entry_path_with_prefixes(
    entry_path: &str,
    target_os: TargetOs,
    data_prefix: &str,
    prefixes: &ReplacementPrefixes,
    rewritten_scripts: &HashSet<String>,
) -> String {
    let Some(rest) = entry_path.strip_prefix(data_prefix) else {
        return entry_path.to_owned();
    };

    if let Some(after) = rest.strip_prefix("purelib/") {
        format!("{}{after}", prefixes.purelib_repl)
    } else if let Some(after) = rest.strip_prefix("platlib/") {
        format!("{}{after}", prefixes.platlib_repl)
    } else if let Some(spath) = rest.strip_prefix("scripts/") {
        if target_os == TargetOs::Windows && rewritten_scripts.contains(spath) {
            format!("{}{spath}.bat", prefixes.scripts_repl)
        } else {
            format!("{}{spath}", prefixes.scripts_repl)
        }
    } else if let Some(after) = rest.strip_prefix("headers/") {
        format!("{}{after}", prefixes.headers_repl)
    } else if let Some(after) = rest.strip_prefix("data/") {
        format!("{}{after}", prefixes.data_repl)
    } else {
        entry_path.to_owned()
    }
}

/// Rewrites a single unescaped `RECORD` entry path (column 0).
pub fn rewrite_entry_path(
    entry_path: &str,
    target_os: TargetOs,
    data_dir_basename: &str,
    rewritten_scripts: &HashSet<String>,
) -> String {
    let data_prefix = format!("{data_dir_basename}/");
    let prefixes = ReplacementPrefixes::for_target_os(target_os);
    rewrite_entry_path_with_prefixes(
        entry_path,
        target_os,
        &data_prefix,
        &prefixes,
        rewritten_scripts,
    )
}

/// Rewrites wheel `RECORD` CSV content from `reader` to `writer` with `\n` line endings.
pub fn rewrite_record_stream<R: Read, W: Write>(
    reader: R,
    writer: W,
    target_os: TargetOs,
    data_dir_basename: &str,
    rewritten_scripts: &HashSet<String>,
) -> io::Result<()> {
    let data_prefix = format!("{data_dir_basename}/");
    let prefixes = ReplacementPrefixes::for_target_os(target_os);

    let mut csv_reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(reader);
    let mut csv_writer = csv::WriterBuilder::new()
        .has_headers(false)
        .flexible(true)
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(writer);

    let mut record = csv::StringRecord::new();
    while csv_reader
        .read_record(&mut record)
        .map_err(io::Error::from)?
    {
        if record.len() < 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "missing delimiter in RECORD entry: {}",
                    record.get(0).unwrap_or_default()
                ),
            ));
        }

        let rewritten_path = rewrite_entry_path_with_prefixes(
            &record[0],
            target_os,
            &data_prefix,
            &prefixes,
            rewritten_scripts,
        );
        csv_writer
            .write_record(std::iter::once(rewritten_path.as_str()).chain(record.iter().skip(1)))
            .map_err(io::Error::from)?;
    }

    csv_writer.flush()
}

/// Rewrites an in-memory `RECORD` string and returns the rewritten UTF-8 string.
pub fn rewrite_record_str(
    input: &str,
    target_os: TargetOs,
    data_dir_basename: &str,
    rewritten_scripts: &HashSet<String>,
) -> io::Result<String> {
    let mut output = Vec::with_capacity(input.len());
    rewrite_record_stream(
        input.as_bytes(),
        &mut output,
        target_os,
        data_dir_basename,
        rewritten_scripts,
    )?;
    String::from_utf8(output).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Rewrites the wheel `RECORD` file at `in_path` to `out_path`.
pub fn rewrite_record_files(
    in_path: &Path,
    out_path: &Path,
    target_os: TargetOs,
    data_dir_basename: &str,
    rewritten_scripts: &HashSet<String>,
) -> io::Result<()> {
    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let in_file = File::open(in_path)?;
    let out_file = File::create(out_path)?;

    rewrite_record_stream(
        in_file,
        out_file,
        target_os,
        data_dir_basename,
        rewritten_scripts,
    )
}

/// Rewrites a wheel `RECORD` file using the provided [`RewriteOptions`].
pub fn rewrite_record(options: &RewriteOptions) -> io::Result<()> {
    rewrite_record_files(
        &options.in_path,
        &options.out_path,
        options.target_os,
        &options.data_dir_basename,
        &options.rewritten_scripts,
    )
}
