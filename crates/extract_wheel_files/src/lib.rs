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
use std::process::ExitCode;

use regex::Regex;
use zip::ZipArchive;
use zip::result::ZipError;

/// Parsed CLI options for `extract_wheel_files`.
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub whl_path: PathBuf,
    pub outdir: PathBuf,
    pub regex: Regex,
}

/// Compiles a start-anchored regular expression (`Python re.match` semantics).
pub fn compile_pattern(regex_pattern: &str) -> Result<Regex, String> {
    let anchored_pattern = format!(r"\A(?:{regex_pattern})");
    Regex::new(&anchored_pattern).map_err(|e| format!("Invalid regex '{regex_pattern}': {e}"))
}

/// Parses CLI arguments (excluding `argv[0]`) using `"extract_wheel_files"` as the program name
/// in usage error messages.
pub fn parse_args<I, T>(args: I) -> Result<ExtractOptions, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    parse_args_with_program("extract_wheel_files", args)
}

/// Parses CLI arguments (excluding `argv[0]`), formatting usage errors with `program_name`.
pub fn parse_args_with_program<I, T>(program_name: &str, args: I) -> Result<ExtractOptions, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    use lexopt::prelude::*;

    let usage = || format!("Usage: {program_name} <wheel> <out_dir> [regex_pattern]");

    let mut parser = lexopt::Parser::from_args(args);
    let mut positionals: Vec<OsString> = Vec::with_capacity(3);
    while let Some(arg) = parser.next().map_err(|_| usage())? {
        match arg {
            Value(val) => positionals.push(val),
            Short(_) | Long(_) => return Err(usage()),
        }
    }

    if positionals.len() != 2 && positionals.len() != 3 {
        return Err(usage());
    }

    let mut iter = positionals.into_iter();
    let whl_path = PathBuf::from(iter.next().unwrap());
    let outdir = PathBuf::from(iter.next().unwrap());
    let regex_pattern = match iter.next() {
        Some(raw) => raw
            .into_string()
            .map_err(|os| format!("Invalid UTF-8 in regex pattern: {os:?}"))?,
        None => String::new(),
    };
    let regex = compile_pattern(&regex_pattern)?;

    Ok(ExtractOptions {
        whl_path,
        outdir,
        regex,
    })
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), extracts matching wheel files,
/// and returns the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = match parse_args(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = run(&options) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Executes the full `extract_wheel_files` workflow for the given options.
pub fn run(options: &ExtractOptions) -> Result<(), String> {
    let record = get_record(&options.whl_path)?;
    let files = get_files(&record, &options.regex);
    extract_files(&options.whl_path, &files, &options.outdir)
}

/// Parses CSV `RECORD` bytes and returns the list of file paths (column 0).
pub fn parse_record(record_bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(record_bytes);
    let mut paths = Vec::new();
    for result in reader.records() {
        let record = result.map_err(|e| format!("Failed to parse RECORD CSV: {e}"))?;
        if let Some(first) = record.get(0)
            && !first.is_empty()
        {
            paths.push(first.to_owned());
        }
    }
    Ok(paths)
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
            "Failed to read {record_name} from {}: {e}",
            whl_path.display(),
        )
    })?;
    let mut record_bytes = Vec::new();
    entry.read_to_end(&mut record_bytes).map_err(|e| {
        format!(
            "Failed to read {record_name} from {}: {e}",
            whl_path.display(),
        )
    })?;

    parse_record(&record_bytes)
}

/// Filters `whl_record` entries by a pre-compiled `regex`.
pub fn get_files<I, S>(whl_record: I, regex: &Regex) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    whl_record
        .into_iter()
        .filter(|entry| regex.is_match(entry.as_ref()))
        .map(|entry| entry.as_ref().to_owned())
        .collect()
}

/// Extracts `files` from `whl_path` into `outdir`.
pub fn extract_files<I, S>(whl_path: &Path, files: I, outdir: &Path) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    fs::create_dir_all(outdir).map_err(|e| {
        format!(
            "Failed to create output directory {}: {e}",
            outdir.display(),
        )
    })?;

    let mut archive = open_wheel_archive(whl_path)?;
    for file in files {
        let entry_name = file.as_ref();
        let mut entry = archive.by_name(entry_name).map_err(|e| match e {
            ZipError::FileNotFound => format!(
                "Entry '{entry_name}' not found in wheel {}",
                whl_path.display()
            ),
            other => format!(
                "Failed to open '{entry_name}' in {}: {other}",
                whl_path.display(),
            ),
        })?;

        let dest_path = sanitize_extract_path(outdir, entry_name);
        if entry_name.ends_with('/') || entry.is_dir() {
            fs::create_dir_all(&dest_path)
                .map_err(|e| format!("Failed to create directory {}: {e}", dest_path.display()))?;
            continue;
        }

        if let Some(parent) = dest_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
        }

        let mut out_file = File::create(&dest_path)
            .map_err(|e| format!("Failed to create {}: {e}", dest_path.display()))?;
        io::copy(&mut entry, &mut out_file).map_err(|e| {
            format!(
                "Failed to extract '{entry_name}' to {}: {e}",
                dest_path.display(),
            )
        })?;
    }

    Ok(())
}

fn open_wheel_archive(whl_path: &Path) -> Result<ZipArchive<BufReader<File>>, String> {
    let file = File::open(whl_path)
        .map_err(|_| format!("{} is not a valid zip file", whl_path.display()))?;
    ZipArchive::new(BufReader::new(file))
        .map_err(|_| format!("{} is not a valid zip file", whl_path.display()))
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
