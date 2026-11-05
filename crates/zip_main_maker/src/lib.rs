//! Library for creating the `__main__.py` for a zipapp by populating a template
//! and computing a SHA-256 hash of the application files from a manifest.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lexopt::prelude::*;
use sha2::{Digest, Sha256};

pub const BLOCK_SIZE: usize = 256 * 1024;
pub const APP_HASH_PLACEHOLDER: &str = "%APP_HASH%";

/// Computes the SHA-256 hex digest of the application inputs listed in `manifest_path`.
pub fn compute_inputs_hash(manifest_path: &Path) -> io::Result<String> {
    let content = fs::read_to_string(manifest_path)?;
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let mut manifest_lines: Vec<&str> = normalized.lines().collect();

    // Sort lines for determinism. Hash the paths (to capture structure) and the content.
    manifest_lines.sort();

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BLOCK_SIZE];

    for line in manifest_lines {
        let (type_, rest) = line.split_once('|').unwrap_or((line, ""));
        hasher.update(rest.as_bytes());

        if type_ == "rf-empty" {
            continue;
        }
        if type_ == "symlink" {
            // The symlink path and the target it points to
            // are captured by hashing the entire line above.
            continue;
        }

        let is_symlink_str = rest.split_once('|').map_or(rest, |(first, _)| first);
        let path = rest.rsplit_once('|').map_or(rest, |(_, last)| last);

        let is_symlink = if is_symlink_str == "-1" {
            !Path::new(path).exists()
        } else {
            is_symlink_str == "1"
        };

        if is_symlink {
            let link_target = fs::read_link(path)?;
            hasher.update(link_target.to_string_lossy().as_bytes());
        } else {
            let mut file = File::open(path)?;
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
            }
        }
    }

    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Builds the ordered substitution map starting with `%APP_HASH% -> app_hash` and
/// applying each `cli_substitutions` entry in order (updating existing keys in place
/// to match Python `dict` insertion/update order).
pub fn build_substitutions(
    app_hash: &str,
    cli_substitutions: &[(String, String)],
) -> Vec<(String, String)> {
    let mut result = vec![(APP_HASH_PLACEHOLDER.to_string(), app_hash.to_string())];
    for (key, val) in cli_substitutions {
        if let Some(existing) = result.iter_mut().find(|(k, _)| k == key) {
            existing.1 = val.clone();
        } else {
            result.push((key.clone(), val.clone()));
        }
    }
    result
}

/// Reads `template_path`, applies `substitutions` in order, and writes the result to `output_path`.
pub fn expand_template(
    template_path: &Path,
    output_path: &Path,
    substitutions: &[(String, String)],
) -> io::Result<()> {
    let mut content = fs::read_to_string(template_path)?;

    for (key, val) in substitutions {
        content = content.replace(key.as_str(), val.as_str());
    }

    if let Some(parent) = output_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    fs::write(output_path, content)
}

/// Executes `zip_main_maker` with the given `Options`.
pub fn generate_zip_main(options: &Options) -> io::Result<()> {
    let app_hash = compute_inputs_hash(&options.hash_files_manifest)?;
    let substitutions = build_substitutions(&app_hash, &options.substitutions);
    expand_template(&options.template, &options.output, &substitutions)
}

/// Parsed command-line options for `zip_main_maker`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub template: PathBuf,
    pub output: PathBuf,
    pub substitutions: Vec<(String, String)>,
    pub hash_files_manifest: PathBuf,
}

fn parse_substitution(raw: &str) -> Result<(String, String), String> {
    let (key, val) = raw
        .split_once('=')
        .ok_or_else(|| format!("invalid --substitution value (expected KEY=VALUE): {raw}"))?;
    Ok((key.to_string(), val.to_string()))
}

/// Parses command-line arguments (excluding `argv[0]`) into `Options`.
pub fn parse_args<I, T>(args: I) -> Result<Options, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let expanded = argfile::expand_args_from(
        args.into_iter().map(Into::into),
        argfile::parse_fromfile,
        argfile::PREFIX,
    )
    .map_err(|e| format!("failed to read flagfile: {e}"))?;

    let mut template: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut substitutions: Vec<(String, String)> = Vec::new();
    let mut hash_files_manifest: Option<PathBuf> = None;

    let mut parser = lexopt::Parser::from_args(expanded);
    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        match arg {
            Short('h') | Long("help") => {
                return Err("Usage: zip_main_maker --template <path> --output <path> \
                     [--substitution <K=V> ...] --hash_files_manifest <path>"
                    .to_string());
            }
            Long("template") => {
                let val = parser.value().map_err(|e| e.to_string())?;
                template = Some(PathBuf::from(val));
            }
            Long("output") => {
                let val = parser.value().map_err(|e| e.to_string())?;
                output = Some(PathBuf::from(val));
            }
            Long("substitution") => {
                let sub = parser
                    .value()
                    .map_err(|e| e.to_string())?
                    .parse_with(parse_substitution)
                    .map_err(|e| e.to_string())?;
                substitutions.push(sub);
            }
            Long("hash_files_manifest") => {
                let val = parser.value().map_err(|e| e.to_string())?;
                hash_files_manifest = Some(PathBuf::from(val));
            }
            _ => return Err(arg.unexpected().to_string()),
        }
    }

    let template = template.ok_or_else(|| "missing required argument: --template".to_string())?;
    let output = output.ok_or_else(|| "missing required argument: --output".to_string())?;
    let hash_files_manifest = hash_files_manifest
        .ok_or_else(|| "missing required argument: --hash_files_manifest".to_string())?;

    Ok(Options {
        template,
        output,
        substitutions,
        hash_files_manifest,
    })
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), generates `__main__.py`, and
/// returns the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = match parse_args(args) {
        Ok(opts) => opts,
        Err(msg) => {
            eprintln!("zip_main_maker: error: {msg}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = generate_zip_main(&options) {
        eprintln!("zip_main_maker: error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
