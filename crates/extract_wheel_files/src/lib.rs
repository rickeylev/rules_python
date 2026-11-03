//! Library for extracting files from a Python wheel (`.whl`) archive matching a regex pattern.
//!
//! Usage:
//!   `extract_wheel_files <wheel> <out_dir> [regex_pattern]`
//!
//! Reads the `.dist-info/RECORD` file from `<wheel>`, filters column 0 file paths using
//! start-anchored regex matching (`re.match` semantics), and extracts matching files into
//! `<out_dir>`.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use regex::Regex;
use zip::result::ZipError;
use zip::ZipArchive;

/// Parsed CLI options for `extract_wheel_files`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractOptions {
    pub whl_path: PathBuf,
    pub outdir: PathBuf,
    pub regex_pattern: String,
}

/// Parses CLI arguments (excluding `argv[0]`) using `"extract_wheel_files"` as the program name
/// in usage error messages.
pub fn parse_args<I, S>(args: I) -> Result<ExtractOptions, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    parse_args_with_program("extract_wheel_files", args)
}

/// Parses CLI arguments (excluding `argv[0]`), formatting usage errors with `program_name`.
pub fn parse_args_with_program<I, S>(program_name: &str, args: I) -> Result<ExtractOptions, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let collected: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    if collected.len() != 2 && collected.len() != 3 {
        return Err(format!(
            "Usage: {} <wheel> <out_dir> [regex_pattern]",
            program_name
        ));
    }

    let whl_path = PathBuf::from(&collected[0]);
    let outdir = PathBuf::from(&collected[1]);
    let regex_pattern = if collected.len() == 3 {
        collected[2].clone()
    } else {
        String::new()
    };

    Ok(ExtractOptions {
        whl_path,
        outdir,
        regex_pattern,
    })
}

fn open_wheel_archive(whl_path: &Path) -> Result<ZipArchive<BufReader<File>>, String> {
    let file = File::open(whl_path)
        .map_err(|_| format!("{} is not a valid zip file", whl_path.display()))?;
    ZipArchive::new(BufReader::new(file))
        .map_err(|_| format!("{} is not a valid zip file", whl_path.display()))
}

/// Parses a single CSV line using Python `csv.reader` default dialect (`delimiter=','`,
/// `quotechar='"'`, double-quote escaping `""`).
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut chars = line.chars().peekable();

    loop {
        match chars.peek() {
            None => break,
            Some(&'"') => {
                chars.next(); // consume opening quote
                let mut field = String::new();
                while let Some(c) = chars.next() {
                    if c == '"' {
                        if chars.peek() == Some(&'"') {
                            chars.next();
                            field.push('"');
                        } else {
                            // End of quoted section; consume any characters until comma or EOL.
                            for rem in chars.by_ref() {
                                if rem == ',' {
                                    break;
                                }
                            }
                            break;
                        }
                    } else {
                        field.push(c);
                    }
                }
                fields.push(field);
            }
            Some(_) => {
                let mut field = String::new();
                for c in chars.by_ref() {
                    if c == ',' {
                        break;
                    }
                    field.push(c);
                }
                fields.push(field);
            }
        }
    }

    fields
}

/// Reads the `.dist-info/RECORD` file from `whl_path` and returns the list of file paths (column 0).
pub fn get_record(whl_path: &Path) -> Result<Vec<String>, String> {
    let mut archive = open_wheel_archive(whl_path)?;
    let record_names: Vec<String> = archive
        .file_names()
        .filter(|name| name.ends_with(".dist-info/RECORD"))
        .map(str::to_owned)
        .collect();

    if record_names.len() != 1 {
        return Err(format!(
            "{} doesn't contain exactly one .dist-info/RECORD",
            whl_path.display()
        ));
    }

    let record_name = &record_names[0];
    let mut entry = archive.by_name(record_name).map_err(|e| {
        format!(
            "Failed to read {} from {}: {}",
            record_name,
            whl_path.display(),
            e
        )
    })?;
    let mut record_text = String::new();
    entry.read_to_string(&mut record_text).map_err(|e| {
        format!(
            "Failed to read {} from {}: {}",
            record_name,
            whl_path.display(),
            e
        )
    })?;

    let mut paths = Vec::new();
    for line in record_text.lines() {
        if line.is_empty() {
            continue;
        }
        let row = parse_csv_line(line);
        if let Some(first) = row.into_iter().next() {
            paths.push(first);
        }
    }

    Ok(paths)
}

/// Filters `whl_record` entries by `regex_pattern` using Python `re.match` (start-anchored)
/// semantics.
pub fn get_files<I, S>(whl_record: I, regex_pattern: &str) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let anchored_pattern = format!(r"\A(?:{})", regex_pattern);
    let regex = Regex::new(&anchored_pattern)
        .map_err(|e| format!("Invalid regex '{}': {}", regex_pattern, e))?;

    let mut matched = Vec::new();
    for entry in whl_record {
        let s = entry.as_ref();
        if regex.is_match(s) {
            matched.push(s.to_string());
        }
    }
    Ok(matched)
}

/// Extracts `files` from `whl_path` into `outdir`.
pub fn extract_files<I, S>(whl_path: &Path, files: I, outdir: &Path) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    fs::create_dir_all(outdir).map_err(|e| {
        format!(
            "Failed to create output directory {}: {}",
            outdir.display(),
            e
        )
    })?;

    let mut archive = open_wheel_archive(whl_path)?;
    for file in files {
        let entry_name = file.as_ref();
        let mut entry = archive.by_name(entry_name).map_err(|e| match e {
            ZipError::FileNotFound => format!(
                "Entry '{}' not found in wheel {}",
                entry_name,
                whl_path.display()
            ),
            other => format!(
                "Failed to open '{}' in {}: {}",
                entry_name,
                whl_path.display(),
                other
            ),
        })?;

        let dest_path = sanitize_extract_path(outdir, entry_name);
        if entry_name.ends_with('/') || entry.is_dir() {
            fs::create_dir_all(&dest_path).map_err(|e| {
                format!("Failed to create directory {}: {}", dest_path.display(), e)
            })?;
            continue;
        }

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
        }

        let mut out_file = File::create(&dest_path)
            .map_err(|e| format!("Failed to create {}: {}", dest_path.display(), e))?;
        io::copy(&mut entry, &mut out_file).map_err(|e| {
            format!(
                "Failed to extract '{}' to {}: {}",
                entry_name,
                dest_path.display(),
                e
            )
        })?;
    }

    Ok(())
}

/// Executes the full `extract_wheel_files` workflow for the given options.
pub fn run(options: &ExtractOptions) -> Result<(), String> {
    let record = get_record(&options.whl_path)?;
    let files = get_files(&record, &options.regex_pattern)?;
    extract_files(&options.whl_path, &files, &options.outdir)
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), extracts matching wheel files,
/// and returns the process exit code.
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
            eprintln!("{}", msg);
            return 1;
        }
    };
    if let Err(err) = run(&options) {
        eprintln!("{}", err);
        return 1;
    }
    0
}

fn sanitize_extract_path(outdir: &Path, entry_name: &str) -> PathBuf {
    let mut dest = outdir.to_path_buf();
    for part in entry_name.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." {
            continue;
        }
        dest.push(part);
    }
    dest
}
