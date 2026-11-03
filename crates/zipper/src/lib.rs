//! Library supporting creating deterministic zipapp archives from a manifest.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File, Metadata};
use std::io::{self, BufWriter, Seek, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter, ZIP64_BYTES_THR};

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

/// A parsed line from a zipper manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub entry_type: EntryType,
    pub is_symlink_str: String,
    pub zip_path: String,
    pub content_path: Option<String>,
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
    let rel = if legacy_external_runfiles && path.starts_with("external/") {
        path["external/".len()..].to_string()
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
            if let Some(&last) = out.last() {
                if last != ".." {
                    out.pop();
                    continue;
                }
            }
            out.push("..");
        } else {
            out.push(part);
        }
    }
    out
}

fn posix_dirname(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(0) => "/",
        Some(idx) => &path[..idx],
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
            is_symlink_str: (*is_symlink_str).to_string(),
            zip_path: (*zip_path).to_string(),
            content_path: Some((*content_path).to_string()),
        },
        ("rf-empty", [_, runfile_path]) => ManifestEntry {
            entry_type: EntryType::RfEmpty,
            is_symlink_str: "0".to_string(),
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
            is_symlink_str: (*is_symlink_str).to_string(),
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
            is_symlink_str: (*is_symlink_str).to_string(),
            zip_path: unix_join(&[runfiles_dir, workspace_name, runfile_path]),
            content_path: Some((*content_path).to_string()),
        },
        ("rf-root-symlink", [_, is_symlink_str, runfile_path, content_path]) => ManifestEntry {
            entry_type: EntryType::RfRootSymlink,
            is_symlink_str: (*is_symlink_str).to_string(),
            zip_path: unix_join(&[runfiles_dir, runfile_path]),
            content_path: Some((*content_path).to_string()),
        },
        ("symlink", [_, runfile_path, link_to_rf_path]) => {
            let zip_path = unix_join(&[runfiles_dir, runfile_path]);
            let link_to = unix_join(&[runfiles_dir, link_to_rf_path]);
            let content_path = relative_path(&link_to, posix_dirname(&zip_path));
            ManifestEntry {
                entry_type: EntryType::Symlink,
                is_symlink_str: "2".to_string(),
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
                        "{}\nError processing line {}: {}",
                        e,
                        line_idx + 1,
                        line.trim()
                    ),
                ));
            }
        }
    }

    // Sort symlink entries first so they have precedence, then sort by zip path.
    entries.sort_by(|a, b| {
        let a_prio = if a.entry_type == EntryType::Symlink {
            0
        } else {
            1
        };
        let b_prio = if b.entry_type == EntryType::Symlink {
            0
        } else {
            1
        };
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
    is_symlink_str: &str,
) -> io::Result<Option<String>> {
    if is_symlink_str == "1" {
        let target = fs::read_link(content_path)?;
        return Ok(Some(target.to_string_lossy().into_owned()));
    }
    if is_symlink_str != "0" || !is_symlink_path(content_path) {
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
            format!("Missing content_path for entry {}", zip_path),
        )
    })?;
    let content_path = Path::new(content_path_str);

    let effective_is_symlink = if entry.is_symlink_str == "-1" {
        if fs::metadata(content_path).is_err() {
            "1"
        } else {
            "0"
        }
    } else {
        entry.is_symlink_str.as_str()
    };

    if let Some(symlink_target) = source_symlink_target(content_path, effective_is_symlink)? {
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

    if let Some(parent) = options.output_zip.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
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

/// Parses command-line arguments (excluding `argv[0]`) into `ZipOptions`.
pub fn parse_args<I, S>(args: I) -> Result<ZipOptions, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut positional = Vec::new();
    let mut compress_level: u32 = 0;
    let mut workspace_name = String::new();
    let mut legacy_external_runfiles = false;
    let mut runfiles_dir = "runfiles".to_string();
    let mut platform_pathsep = std::path::MAIN_SEPARATOR_STR.to_string();

    let mut iter = args.into_iter();
    while let Some(arg_item) = iter.next() {
        let arg = arg_item.as_ref();
        if arg == "-h" || arg == "--help" {
            return Err("Usage: zipper [OPTIONS] <manifest> <output>".to_string());
        } else if let Some(val) = arg.strip_prefix("--compression=") {
            compress_level = val
                .parse::<u32>()
                .map_err(|_| format!("invalid --compression value: {}", val))?;
        } else if arg == "--compression" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --compression".to_string())?;
            compress_level = val
                .as_ref()
                .parse::<u32>()
                .map_err(|_| format!("invalid --compression value: {}", val.as_ref()))?;
        } else if let Some(val) = arg.strip_prefix("--workspace-name=") {
            workspace_name = val.to_string();
        } else if arg == "--workspace-name" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --workspace-name".to_string())?;
            workspace_name = val.as_ref().to_string();
        } else if let Some(val) = arg.strip_prefix("--legacy-external-runfiles=") {
            legacy_external_runfiles = match val {
                "0" => false,
                "1" => true,
                _ => {
                    return Err(format!(
                        "invalid --legacy-external-runfiles value (expected 0 or 1): {}",
                        val
                    ))
                }
            };
        } else if arg == "--legacy-external-runfiles" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --legacy-external-runfiles".to_string())?;
            legacy_external_runfiles = match val.as_ref() {
                "0" => false,
                "1" => true,
                other => {
                    return Err(format!(
                        "invalid --legacy-external-runfiles value (expected 0 or 1): {}",
                        other
                    ))
                }
            };
        } else if let Some(val) = arg.strip_prefix("--runfiles-dir=") {
            runfiles_dir = val.to_string();
        } else if arg == "--runfiles-dir" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --runfiles-dir".to_string())?;
            runfiles_dir = val.as_ref().to_string();
        } else if let Some(val) = arg.strip_prefix("--target-platform-pathsep=") {
            platform_pathsep = val.to_string();
        } else if arg == "--target-platform-pathsep" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --target-platform-pathsep".to_string())?;
            platform_pathsep = val.as_ref().to_string();
        } else if arg.starts_with('-') {
            return Err(format!("unknown option: {}", arg));
        } else {
            positional.push(arg.to_string());
        }
    }

    if positional.len() != 2 {
        return Err(format!(
            "expected 2 positional arguments (<manifest> <output>), got {}",
            positional.len()
        ));
    }

    Ok(ZipOptions {
        manifest_path: PathBuf::from(&positional[0]),
        output_zip: PathBuf::from(&positional[1]),
        compress_level,
        workspace_name,
        legacy_external_runfiles,
        runfiles_dir,
        platform_pathsep,
    })
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), creates the zip archive, and
/// returns the process exit code.
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
            eprintln!("zipper: error: {}", msg);
            return 1;
        }
    };
    if let Err(err) = create_zip(&options) {
        eprintln!(
            "zipper: error creating zip {}: {}",
            options.output_zip.display(),
            err
        );
        return 1;
    }
    0
}
