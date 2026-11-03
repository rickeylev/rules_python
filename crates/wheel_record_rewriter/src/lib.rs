//! Rewrites a wheel's RECORD file to reflect its final installed layout.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Parsed command-line arguments for rewriting a wheel `RECORD` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteOptions {
    pub in_path: PathBuf,
    pub out_path: PathBuf,
    pub target_os: String,
    pub data_dir_basename: String,
    pub rewritten_scripts: HashSet<String>,
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
    fn for_target_os(target_os: &str) -> Self {
        if target_os == "windows" {
            Self {
                data_repl: "../../",
                headers_repl: "../../Include/",
                platlib_repl: "",
                purelib_repl: "",
                scripts_repl: "../../Scripts/",
            }
        } else {
            Self {
                data_repl: "../../../",
                headers_repl: "../../../include/",
                platlib_repl: "",
                purelib_repl: "",
                scripts_repl: "../../../bin/",
            }
        }
    }
}

/// Parses positional command-line arguments (excluding `argv[0]`).
///
/// Expected positional arguments:
/// `<in_path> <out_path> <target_os> <data_dir_basename> [rewritten_scripts...]`
pub fn parse_args(args: &[String]) -> Result<RewriteOptions, String> {
    if args.len() < 4 {
        return Err(
            "Usage: wheel_record_rewriter <in_path> <out_path> <target_os> <data_dir_basename> [rewritten_scripts...]"
                .to_string(),
        );
    }

    let in_path = PathBuf::from(&args[0]);
    let out_path = PathBuf::from(&args[1]);
    let target_os = args[2].clone();
    let data_dir_basename = args[3].clone();
    let rewritten_scripts: HashSet<String> = args[4..].iter().cloned().collect();

    Ok(RewriteOptions {
        in_path,
        out_path,
        target_os,
        data_dir_basename,
        rewritten_scripts,
    })
}

fn rewrite_line_with_prefixes(
    raw_line: &str,
    is_windows: bool,
    data_prefix: &str,
    quoted_data_prefix: &str,
    prefixes: &ReplacementPrefixes,
    rewritten_scripts: &HashSet<String>,
) -> io::Result<String> {
    let line = raw_line.trim_end_matches(['\r', '\n']);

    let (quote, rest) = if let Some(rest) = line.strip_prefix(quoted_data_prefix) {
        ("\"", rest)
    } else if let Some(rest) = line.strip_prefix(data_prefix) {
        ("", rest)
    } else {
        return Ok(line.to_string());
    };

    if let Some(after) = rest.strip_prefix("purelib/") {
        Ok(format!("{}{}{}", quote, prefixes.purelib_repl, after))
    } else if let Some(after) = rest.strip_prefix("platlib/") {
        Ok(format!("{}{}{}", quote, prefixes.platlib_repl, after))
    } else if let Some(entry) = rest.strip_prefix("scripts/") {
        if is_windows {
            let delimiter = if quote == "\"" { '"' } else { ',' };
            let idx = entry.find(delimiter).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "missing delimiter {:?} in scripts RECORD entry: {}",
                        delimiter, line
                    ),
                )
            })?;
            let (spath, suffix) = (&entry[..idx], &entry[idx..]);
            if rewritten_scripts.contains(spath) {
                Ok(format!(
                    "{}{}{}.bat{}",
                    quote, prefixes.scripts_repl, spath, suffix
                ))
            } else {
                Ok(format!(
                    "{}{}{}{}",
                    quote, prefixes.scripts_repl, spath, suffix
                ))
            }
        } else {
            Ok(format!("{}{}{}", quote, prefixes.scripts_repl, entry))
        }
    } else if let Some(after) = rest.strip_prefix("headers/") {
        Ok(format!("{}{}{}", quote, prefixes.headers_repl, after))
    } else if let Some(after) = rest.strip_prefix("data/") {
        Ok(format!("{}{}{}", quote, prefixes.data_repl, after))
    } else {
        Ok(line.to_string())
    }
}

/// Rewrites a single line of a wheel `RECORD` file (stripping trailing `\r`/`\n`).
pub fn rewrite_line(
    raw_line: &str,
    target_os: &str,
    data_dir_basename: &str,
    rewritten_scripts: &HashSet<String>,
) -> io::Result<String> {
    let data_prefix = format!("{}/", data_dir_basename);
    let quoted_data_prefix = format!("\"{}", data_prefix);
    let prefixes = ReplacementPrefixes::for_target_os(target_os);
    rewrite_line_with_prefixes(
        raw_line,
        target_os == "windows",
        &data_prefix,
        &quoted_data_prefix,
        &prefixes,
        rewritten_scripts,
    )
}

/// Rewrites wheel `RECORD` content from a reader to a writer with `\n` line endings.
pub fn rewrite_record_stream<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    target_os: &str,
    data_dir_basename: &str,
    rewritten_scripts: &HashSet<String>,
) -> io::Result<()> {
    let data_prefix = format!("{}/", data_dir_basename);
    let quoted_data_prefix = format!("\"{}", data_prefix);
    let prefixes = ReplacementPrefixes::for_target_os(target_os);
    let is_windows = target_os == "windows";

    let mut raw_line = String::new();
    loop {
        raw_line.clear();
        let bytes_read = reader.read_line(&mut raw_line)?;
        if bytes_read == 0 {
            break;
        }
        let rewritten = rewrite_line_with_prefixes(
            &raw_line,
            is_windows,
            &data_prefix,
            &quoted_data_prefix,
            &prefixes,
            rewritten_scripts,
        )?;
        writer.write_all(rewritten.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.flush()
}

/// Rewrites an in-memory `RECORD` string and returns the rewritten UTF-8 string.
pub fn rewrite_record_str(
    input: &str,
    target_os: &str,
    data_dir_basename: &str,
    rewritten_scripts: &HashSet<String>,
) -> io::Result<String> {
    let mut output = Vec::with_capacity(input.len());
    rewrite_record_stream(
        io::Cursor::new(input.as_bytes()),
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
    target_os: &str,
    data_dir_basename: &str,
    rewritten_scripts: &HashSet<String>,
) -> io::Result<()> {
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let in_file = File::open(in_path)?;
    let out_file = File::create(out_path)?;
    let reader = BufReader::new(in_file);
    let writer = BufWriter::new(out_file);

    rewrite_record_stream(
        reader,
        writer,
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
        &options.target_os,
        &options.data_dir_basename,
        &options.rewritten_scripts,
    )
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), rewrites the wheel `RECORD`
/// file, and returns the process exit code.
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
            eprintln!("wheel_record_rewriter: error: {}", msg);
            return 1;
        }
    };
    if let Err(err) = rewrite_record(&options) {
        eprintln!(
            "wheel_record_rewriter: error rewriting {} to {}: {}",
            options.in_path.display(),
            options.out_path.display(),
            err
        );
        return 1;
    }
    0
}
