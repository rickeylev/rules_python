//! Library supporting creating deterministic zipapp archives from a manifest.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File, Metadata};
use std::io::{self, BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lexopt::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZIP64_BYTES_THR, ZipWriter};

pub const BLOCK_SIZE: usize = 256 * 1024;

/// Entry types supported in a zipper manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    Regular,
    RfEmpty,
    RfFile,
    RfSymlink,
    RfRootSymlink,
    Symlink,
}

/// Symlink handling mode parsed from a zipper manifest entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymlinkMode {
    /// `"0"`: store as regular file unless the source itself is a relative symlink.
    Follow,
    /// `"1"`: store as symlink by reading the link target on disk.
    Symlink,
    /// `"-1"`: infer symlink mode (`Symlink` if dangling on disk, else `Follow`).
    Auto,
    /// Direct symlink entry (`symlink|<rf_path>|<link_to_rf_path>`).
    Direct,
}

/// A parsed line from a zipper manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub entry_type: EntryType,
    pub symlink_mode: SymlinkMode,
    pub zip_path: String,
    pub content_path: Option<String>,
}

/// Joins path components with `/`.
pub fn unix_join(parts: &[&str]) -> String {
    parts.join("/")
}

/// Computes the zip path for a runfile entry.
pub fn get_zip_runfiles_path(
    path: &str,
    workspace_name: &str,
    legacy_external_runfiles: bool,
    runfiles_dir: &str,
) -> String {
    let rel = if legacy_external_runfiles && let Some(stripped) = path.strip_prefix("external/") {
        stripped.to_string()
    } else if let Some(stripped) = path.strip_prefix("../") {
        stripped.to_string()
    } else {
        unix_join(&[workspace_name, path])
    };
    unix_join(&[runfiles_dir, &rel])
}

fn normalize_components(path: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            if let Some(&last) = out.last()
                && last != ".."
            {
                out.pop();
                continue;
            }
            out.push("..");
        } else {
            out.push(part);
        }
    }
    out
}

fn posix_dirname(path: &str) -> &str {
    match path.rsplit_once(['/', '\\']) {
        Some(("", _)) => "/",
        Some((dir, _)) => dir,
        None => "",
    }
}

/// Computes a relative path from `start` directory to `target` using `/` separators,
/// matching `os.path.relpath(target, start=start)`.
pub fn relative_path(target: &str, start: &str) -> String {
    let start_parts = normalize_components(start);
    let target_parts = normalize_components(target);
    let common = start_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = Vec::new();
    for _ in common..start_parts.len() {
        result.push("..");
    }
    for &part in &target_parts[common..] {
        result.push(part);
    }
    if result.is_empty() {
        ".".to_string()
    } else {
        result.join("/")
    }
}

/// Converts a symlink target path to the target-platform format.
pub fn convert_symlink_target(path: &str, platform_pathsep: &str) -> String {
    if platform_pathsep == "/" {
        path.replace('\\', "/")
    } else {
        path.replace('/', "\\")
    }
}

/// Normalizes zip entry paths to use forward slashes.
pub fn normalize_zip_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn parse_symlink_mode(raw: &str, line_idx: usize, line: &str) -> io::Result<SymlinkMode> {
    match raw {
        "0" => Ok(SymlinkMode::Follow),
        "1" => Ok(SymlinkMode::Symlink),
        "-1" => Ok(SymlinkMode::Auto),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "Error: Invalid symlink mode '{}' at line {}: {}",
                raw,
                line_idx + 1,
                line
            ),
        )),
    }
}

/// Parses a single line of a manifest file.
pub fn parse_entry(
    line: &str,
    line_idx: usize,
    workspace_name: &str,
    legacy_external_runfiles: bool,
    runfiles_dir: &str,
) -> io::Result<Option<ManifestEntry>> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }

    let parts: Vec<&str> = line.split('|').collect();
    let type_str = parts[0];

    let entry = match (type_str, parts.as_slice()) {
        ("regular", [_, is_symlink_str, zip_path, content_path]) => ManifestEntry {
            entry_type: EntryType::Regular,
            symlink_mode: parse_symlink_mode(is_symlink_str, line_idx, line)?,
            zip_path: (*zip_path).to_string(),
            content_path: Some((*content_path).to_string()),
        },
        ("rf-empty", [_, runfile_path]) => ManifestEntry {
            entry_type: EntryType::RfEmpty,
            symlink_mode: SymlinkMode::Follow,
            zip_path: get_zip_runfiles_path(
                runfile_path,
                workspace_name,
                legacy_external_runfiles,
                runfiles_dir,
            ),
            content_path: None,
        },
        ("rf-file", [_, is_symlink_str, runfile_path, content_path]) => ManifestEntry {
            entry_type: EntryType::RfFile,
            symlink_mode: parse_symlink_mode(is_symlink_str, line_idx, line)?,
            zip_path: get_zip_runfiles_path(
                runfile_path,
                workspace_name,
                legacy_external_runfiles,
                runfiles_dir,
            ),
            content_path: Some((*content_path).to_string()),
        },
        ("rf-symlink", [_, is_symlink_str, runfile_path, content_path]) => ManifestEntry {
            entry_type: EntryType::RfSymlink,
            symlink_mode: parse_symlink_mode(is_symlink_str, line_idx, line)?,
            zip_path: unix_join(&[runfiles_dir, workspace_name, runfile_path]),
            content_path: Some((*content_path).to_string()),
        },
        ("rf-root-symlink", [_, is_symlink_str, runfile_path, content_path]) => ManifestEntry {
            entry_type: EntryType::RfRootSymlink,
            symlink_mode: parse_symlink_mode(is_symlink_str, line_idx, line)?,
            zip_path: unix_join(&[runfiles_dir, runfile_path]),
            content_path: Some((*content_path).to_string()),
        },
        ("symlink", [_, runfile_path, link_to_rf_path]) => {
            let zip_path = unix_join(&[runfiles_dir, runfile_path]);
            let link_to = unix_join(&[runfiles_dir, link_to_rf_path]);
            let content_path = relative_path(&link_to, posix_dirname(&zip_path));
            ManifestEntry {
                entry_type: EntryType::Symlink,
                symlink_mode: SymlinkMode::Direct,
                zip_path,
                content_path: Some(content_path),
            }
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Error: Unknown entry type or invalid format at line {}: {}",
                    line_idx + 1,
                    line
                ),
            ));
        }
    };

    Ok(Some(entry))
}

/// Reads and sorts all entries from the manifest file at `manifest_path`.
pub fn read_manifest(
    manifest_path: &Path,
    workspace_name: &str,
    legacy_external_runfiles: bool,
    runfiles_dir: &str,
) -> io::Result<Vec<ManifestEntry>> {
    let content = fs::read_to_string(manifest_path)?;
    let mut entries = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        match parse_entry(
            line,
            line_idx,
            workspace_name,
            legacy_external_runfiles,
            runfiles_dir,
        ) {
            Ok(Some(entry)) => entries.push(entry),
            Ok(None) => {}
            Err(e) => {
                return Err(io::Error::new(
                    e.kind(),
                    format!(
                        "{e}\nError processing line {}: {}",
                        line_idx + 1,
                        line.trim()
                    ),
                ));
            }
        }
    }

    // Sort symlink entries first so they have precedence, then sort by zip path.
    entries.sort_by(|a, b| {
        let a_prio = usize::from(a.entry_type != EntryType::Symlink);
        let b_prio = usize::from(b.entry_type != EntryType::Symlink);
        a.zip_path.cmp(&b.zip_path).then(a_prio.cmp(&b_prio))
    });

    Ok(entries)
}

fn is_symlink_path(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn is_abs_path(path: &Path) -> bool {
    if path.is_absolute() {
        return true;
    }
    let s = path.to_string_lossy();
    s.starts_with('/') || s.starts_with('\\')
}

/// Determines whether `content_path` should be stored as a symlink and returns its target.
pub fn source_symlink_target(
    content_path: &Path,
    symlink_mode: SymlinkMode,
) -> io::Result<Option<String>> {
    let resolved_mode = match symlink_mode {
        SymlinkMode::Auto => {
            if fs::metadata(content_path).is_err() {
                SymlinkMode::Symlink
            } else {
                SymlinkMode::Follow
            }
        }
        other => other,
    };

    if resolved_mode == SymlinkMode::Symlink {
        let target = fs::read_link(content_path)?;
        return Ok(Some(target.to_string_lossy().into_owned()));
    }
    if resolved_mode != SymlinkMode::Follow || !is_symlink_path(content_path) {
        return Ok(None);
    }

    let target = fs::read_link(content_path)?;
    if !is_abs_path(&target) {
        return Ok(Some(target.to_string_lossy().into_owned()));
    }

    // Bazel sandboxes expose regular inputs as absolute symlinks. Look through
    // that indirection to detect whether the original source is also a symlink.
    // Only a relative link can be kept: an absolute one points into this
    // machine's Bazel output base (e.g. the _solib links Bazel creates for shared
    // libraries) and would dangle wherever the zipapp runs, so store the file.
    if is_symlink_path(&target) {
        let source_target = fs::read_link(&target)?;
        if !is_abs_path(&source_target) {
            return Ok(Some(source_target.to_string_lossy().into_owned()));
        }
    }
    Ok(None)
}

fn mode_from_metadata(meta: &Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mode() & 0xFFFF
    }
    #[cfg(not(unix))]
    {
        if meta.permissions().readonly() {
            0o100444
        } else {
            0o100666
        }
    }
}

fn base_file_options(compress_level: u32) -> SimpleFileOptions {
    let options = SimpleFileOptions::default().last_modified_time(DateTime::default());
    if compress_level == 0 {
        options.compression_method(CompressionMethod::Stored)
    } else {
        options
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(i64::from(compress_level.min(9))))
    }
}

fn symlink_file_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .last_modified_time(DateTime::default())
        .unix_permissions(0o777)
}

fn write_entry<W: Write + Seek>(
    zf: &mut ZipWriter<W>,
    entry: &ManifestEntry,
    compress_level: u32,
    seen: &mut HashSet<String>,
    platform_pathsep: &str,
) -> io::Result<()> {
    let zip_path = normalize_zip_path(&entry.zip_path);
    if !seen.insert(zip_path.clone()) {
        // Symlink entries are sorted before non-symlink entries and take precedence.
        return Ok(());
    }

    if entry.entry_type == EntryType::RfEmpty {
        let opts = base_file_options(compress_level).unix_permissions(0o644);
        zf.start_file(&zip_path, opts).map_err(io::Error::from)?;
        return Ok(());
    }

    if entry.entry_type == EntryType::Symlink {
        let content_path = entry.content_path.as_deref().unwrap_or("");
        let target = convert_symlink_target(content_path, platform_pathsep);
        zf.add_symlink(&zip_path, target, symlink_file_options())
            .map_err(io::Error::from)?;
        return Ok(());
    }

    let content_path_str = entry.content_path.as_deref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Missing content_path for entry {zip_path}"),
        )
    })?;
    let content_path = Path::new(content_path_str);

    if let Some(symlink_target) = source_symlink_target(content_path, entry.symlink_mode)? {
        let target = convert_symlink_target(&symlink_target, platform_pathsep);
        zf.add_symlink(&zip_path, target, symlink_file_options())
            .map_err(io::Error::from)?;
        Ok(())
    } else {
        let mut src = File::open(content_path)?;
        let meta = src.metadata()?;
        let mode = mode_from_metadata(&meta);
        let opts = base_file_options(compress_level)
            .unix_permissions(mode)
            .large_file(meta.len() >= ZIP64_BYTES_THR);
        zf.start_file(&zip_path, opts).map_err(io::Error::from)?;
        io::copy(&mut src, zf)?;
        Ok(())
    }
}

/// Creates a zip file from a manifest according to `options`.
pub fn create_zip(options: &ZipOptions) -> io::Result<()> {
    let entries = read_manifest(
        &options.manifest_path,
        &options.workspace_name,
        options.legacy_external_runfiles,
        &options.runfiles_dir,
    )?;

    if let Some(parent) = options.output_zip.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let out_file = BufWriter::with_capacity(BLOCK_SIZE, File::create(&options.output_zip)?);
    let mut zf = ZipWriter::new(out_file);
    let mut seen = HashSet::new();

    for entry in &entries {
        write_entry(
            &mut zf,
            entry,
            options.compress_level,
            &mut seen,
            &options.platform_pathsep,
        )?;
    }

    let mut inner = zf.finish().map_err(io::Error::from)?;
    inner.flush()
}

/// Configuration options for creating a zip archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipOptions {
    pub manifest_path: PathBuf,
    pub output_zip: PathBuf,
    pub compress_level: u32,
    pub workspace_name: String,
    pub legacy_external_runfiles: bool,
    pub runfiles_dir: String,
    pub platform_pathsep: String,
}

fn parse_legacy_external_runfiles(val: &str) -> Result<bool, String> {
    match val {
        "0" => Ok(false),
        "1" => Ok(true),
        other => Err(format!(
            "invalid --legacy-external-runfiles value (expected 0 or 1): {other}"
        )),
    }
}

/// Parses command-line arguments (excluding `argv[0]`) into `ZipOptions`.
pub fn parse_args<I, T>(args: I) -> Result<ZipOptions, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut compress_level: u32 = 0;
    let mut workspace_name = String::new();
    let mut legacy_external_runfiles = false;
    let mut runfiles_dir = "runfiles".to_string();
    let mut platform_pathsep = std::path::MAIN_SEPARATOR_STR.to_string();

    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        match arg {
            Short('h') | Long("help") => {
                return Err("Usage: zipper [OPTIONS] <manifest> <output>".to_string());
            }
            Long("compression") => {
                compress_level = parser
                    .value()
                    .map_err(|e| e.to_string())?
                    .parse::<u32>()
                    .map_err(|e| e.to_string())?;
            }
            Long("workspace-name") => {
                workspace_name = parser
                    .value()
                    .map_err(|e| e.to_string())?
                    .string()
                    .map_err(|e| e.to_string())?;
            }
            Long("legacy-external-runfiles") => {
                legacy_external_runfiles = parser
                    .value()
                    .map_err(|e| e.to_string())?
                    .parse_with(parse_legacy_external_runfiles)
                    .map_err(|e| e.to_string())?;
            }
            Long("runfiles-dir") => {
                runfiles_dir = parser
                    .value()
                    .map_err(|e| e.to_string())?
                    .string()
                    .map_err(|e| e.to_string())?;
            }
            Long("target-platform-pathsep") => {
                platform_pathsep = parser
                    .value()
                    .map_err(|e| e.to_string())?
                    .string()
                    .map_err(|e| e.to_string())?;
            }
            Value(val) => {
                positional.push(PathBuf::from(val));
            }
            _ => return Err(arg.unexpected().to_string()),
        }
    }

    let [manifest_path, output_zip]: [PathBuf; 2] =
        positional.try_into().map_err(|v: Vec<_>| {
            format!(
                "expected 2 positional arguments (<manifest> <output>), got {}",
                v.len()
            )
        })?;

    Ok(ZipOptions {
        manifest_path,
        output_zip,
        compress_level,
        workspace_name,
        legacy_external_runfiles,
        runfiles_dir,
        platform_pathsep,
    })
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), creates the zip archive, and
/// returns the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = match parse_args(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("zipper: error: {msg}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = create_zip(&options) {
        eprintln!(
            "zipper: error creating zip {}: {err}",
            options.output_zip.display()
        );
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
