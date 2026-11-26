//! Library for embedding target-specific bootstrap metadata into a native stage-1 bootstrap
//! template binary.

// Errors must be returned, not panicked; see the `rust-lint` config in .bazelrc.
#![deny(clippy::expect_used, clippy::unwrap_used)]

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use lexopt::prelude::*;
use stage1_native_bootstrap_lib::{
    CFG_BUFFER_SIZE, CFG_FOOTER_ZERO_LEN, CFG_HEADER_LEN, CFG_MAGIC_LEN, EOF_TRAILER_LEN,
    EOF_TRAILER_MAGIC, find_magic_offset, has_magic_sentinel,
};

mod flags {
    // keep-sorted start case=no
    pub const HELP: &str = "help";
    pub const HELP_SHORT: char = 'h';
    pub const INTERPRETER_ARG: &str = "interpreter-arg";
    pub const IS_ZIPFILE: &str = "is-zipfile";
    pub const OUTPUT: &str = "output";
    pub const PYTHON_BINARY: &str = "python-binary";
    pub const PYTHON_BINARY_ACTUAL: &str = "python-binary-actual";
    pub const RECREATE_VENV_AT_RUNTIME: &str = "recreate-venv-at-runtime";
    pub const RESOLVE_PYTHON_BINARY_AT_RUNTIME: &str = "resolve-python-binary-at-runtime";
    pub const RUNTIME_VENV_SYMLINK: &str = "runtime-venv-symlink";
    pub const STAGE2_BOOTSTRAP_PATH: &str = "stage2-bootstrap-path";
    pub const TARGET: &str = "target";
    pub const TEMPLATE: &str = "template";
    pub const VENV_REL_SITE_PACKAGES: &str = "venv-rel-site-packages";
    pub const WORKSPACE_NAME: &str = "workspace-name";
    // keep-sorted end
}

mod payload_keys {
    // keep-sorted start case=no
    pub const INTERPRETER_ARG: &str = "interpreter_arg";
    pub const IS_ZIPFILE: &str = "is_zipfile";
    pub const PYTHON_BINARY: &str = "python_binary";
    pub const PYTHON_BINARY_ACTUAL: &str = "python_binary_actual";
    pub const RECREATE_VENV_AT_RUNTIME: &str = "recreate_venv_at_runtime";
    pub const RESOLVE_PYTHON_BINARY_AT_RUNTIME: &str = "resolve_python_binary_at_runtime";
    pub const RUNTIME_VENV_SYMLINK: &str = "runtime_venv_symlink";
    pub const STAGE2_BOOTSTRAP: &str = "stage2_bootstrap";
    pub const TARGET: &str = "target";
    pub const VENV_REL_SITE_PACKAGES: &str = "venv_rel_site_packages";
    pub const WORKSPACE_NAME: &str = "workspace_name";
    // keep-sorted end
}

/// Usage text for `--help`. `lexopt` does not generate help, so this is maintained by hand.
const USAGE: &str = "\
Usage: native_bootstrap_embedder [OPTIONS] --template <PATH> --output <PATH>

Embed target-specific Python bootstrap configuration into a native stage-1
bootstrap template binary.

Options:
  --template <PATH>                            Input native bootstrap template binary (CWD-relative or absolute)
  --output <PATH>                              Output executable binary path (CWD-relative or absolute)
  --stage2-bootstrap-path <RF_ROOT_REL_PATH>   Runfiles-root-relative path to the stage-2 Python script (e.g. _main/pkg/app_stage2.py)
  --python-binary <PATH>                       Python interpreter path: runfiles-root-relative (typically venv bin/python3), absolute, or bare PATH name
  --python-binary-actual <PATH>                Underlying interpreter target symlinked by --python-binary for zip/runtime-venv: runfiles-root-relative, absolute, or bare PATH name
  --is-zipfile <0|1>                           Whether the target is a self-extracting zip (default: 0)
  --recreate-venv-at-runtime <0|1>             Whether stage-1 recreates the venv at runtime (default: 0)
  --resolve-python-binary-at-runtime <0|1>     Whether stage-1 resolves --python-binary-actual at runtime (default: 0)
  --venv-rel-site-packages <VENV_REL_PATH>     Venv-root-relative path to site-packages (e.g. lib/python3.12/site-packages)
  --workspace-name <NAME>                      Bazel workspace name
  --target <LABEL>                             Bazel target label
  --interpreter-arg <ARG>                      Argument from the target's interpreter_args attr passed to Python before stage-2 (repeatable)
  --runtime-venv-symlink <VENV_REL|RF_ROOT_REL>
                                               Runtime venv symlink mapping (repeatable): <venv-root-relative>|<runfiles-root-relative>
  @<FLAGFILE>                                  Read newline-delimited arguments from <FLAGFILE>
  -h, --help                                   Print this help";

fn push_kv(buf: &mut Vec<u8>, key: &str, val: &str) {
    buf.extend_from_slice(key.as_bytes());
    buf.push(b'=');
    buf.extend_from_slice(val.as_bytes());
    buf.push(0);
}

const fn bool_str(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

/// Serializes `options` into the NUL-delimited `key=value` byte payload consumed by the stage-1
/// native bootstrap runtime.
pub fn serialize_cfg_payload(options: &Options) -> Vec<u8> {
    let mut payload = Vec::new();
    push_kv(
        &mut payload,
        payload_keys::STAGE2_BOOTSTRAP,
        &options.stage2_bootstrap_path,
    );
    push_kv(
        &mut payload,
        payload_keys::PYTHON_BINARY,
        &options.python_binary,
    );
    push_kv(
        &mut payload,
        payload_keys::PYTHON_BINARY_ACTUAL,
        &options.python_binary_actual,
    );
    push_kv(
        &mut payload,
        payload_keys::IS_ZIPFILE,
        bool_str(options.is_zipfile),
    );
    push_kv(
        &mut payload,
        payload_keys::RECREATE_VENV_AT_RUNTIME,
        bool_str(options.recreate_venv_at_runtime),
    );
    push_kv(
        &mut payload,
        payload_keys::RESOLVE_PYTHON_BINARY_AT_RUNTIME,
        bool_str(options.resolve_python_binary_at_runtime),
    );
    push_kv(
        &mut payload,
        payload_keys::VENV_REL_SITE_PACKAGES,
        &options.venv_rel_site_packages,
    );
    push_kv(
        &mut payload,
        payload_keys::WORKSPACE_NAME,
        &options.workspace_name,
    );
    push_kv(&mut payload, payload_keys::TARGET, &options.target);
    for arg in &options.interpreter_args {
        push_kv(&mut payload, payload_keys::INTERPRETER_ARG, arg);
    }
    for sym in &options.runtime_venv_symlinks {
        push_kv(
            &mut payload,
            payload_keys::RUNTIME_VENV_SYMLINK,
            &format!("{}|{}", sym.venv_path, sym.link_to_path),
        );
    }
    payload.push(0);
    payload
}

/// Embeds `options` into the raw template binary bytes `tpl_buf`.
///
/// * On Unix templates (ELF/Mach-O), patches the 4 KiB `.data` sentinel buffer in-place.
/// * On Windows PE (`MZ`) templates, appends the variable-sized EOF trailer
///   (`[payload][payload_len: u64_le][EOF_TRAILER_MAGIC]`) so arbitrarily large runtime venv
///   symlink lists fit without resizing PE sections.
///
/// # Errors
///
/// Returns an error if `tpl_buf` does not contain the `CFG_MAGIC` sentinel or if the serialized
/// payload exceeds the 4 KiB in-place buffer capacity (`4,044` bytes) on non-PE templates.
pub fn embed_config_into_binary(tpl_buf: &[u8], options: &Options) -> Result<Vec<u8>> {
    let payload = serialize_cfg_payload(options);
    if tpl_buf.starts_with(b"MZ") {
        if !has_magic_sentinel(tpl_buf) {
            bail!(
                "template binary is missing native bootstrap magic sentinel \
                 (\\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\\0); ensure bootstrap_template is a native \
                 bootstrap binary when --bootstrap_impl=native"
            );
        }
        let payload_len = u64::try_from(payload.len()).context("payload length overflows u64")?;
        let mut out = Vec::with_capacity(tpl_buf.len() + payload.len() + EOF_TRAILER_LEN);
        out.extend_from_slice(tpl_buf);
        out.extend_from_slice(&payload);
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&EOF_TRAILER_MAGIC);
        return Ok(out);
    }

    let Some(magic_offset) = find_magic_offset(tpl_buf) else {
        bail!(
            "template binary is missing native bootstrap magic sentinel \
             (\\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\\0); ensure bootstrap_template is a native \
             bootstrap binary when --bootstrap_impl=native"
        );
    };

    let max_inplace_len = CFG_BUFFER_SIZE - CFG_HEADER_LEN - CFG_FOOTER_ZERO_LEN;
    if payload.len() > max_inplace_len {
        bail!(
            "bootstrap config payload ({} bytes) exceeds 4 KiB in-place buffer capacity \
             ({max_inplace_len} bytes)",
            payload.len()
        );
    }

    let payload_len = u32::try_from(payload.len()).context("payload length overflows u32")?;
    let mut out = tpl_buf.to_vec();
    let cfg_region = &mut out[magic_offset..magic_offset + CFG_BUFFER_SIZE];
    cfg_region[CFG_MAGIC_LEN..].fill(0);
    cfg_region[32..36].copy_from_slice(&payload_len.to_le_bytes());
    cfg_region[CFG_HEADER_LEN..CFG_HEADER_LEN + payload.len()].copy_from_slice(&payload);
    Ok(out)
}

fn set_executable_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(path, perms)
            .with_context(|| format!("setting executable permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Reads `options.template`, embeds `options`, and writes the resulting executable to
/// `options.output`.
///
/// # Errors
///
/// Returns an error if reading `options.template` fails, embedding fails, or creating/writing
/// `options.output` fails.
pub fn embed_bootstrap_file(options: &Options) -> Result<()> {
    let tpl_buf = fs::read(&options.template)
        .with_context(|| format!("reading template {}", options.template.display()))?;
    let out_buf = embed_config_into_binary(&tpl_buf, options)
        .with_context(|| format!("in template {}", options.template.display()))?;

    if let Some(parent) = options.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }

    let mut file = File::create(&options.output)
        .with_context(|| format!("creating output {}", options.output.display()))?;
    file.write_all(&out_buf)
        .with_context(|| format!("writing output {}", options.output.display()))?;
    file.flush()
        .with_context(|| format!("flushing output {}", options.output.display()))?;
    drop(file);

    set_executable_permissions(&options.output)?;
    Ok(())
}

/// A `(venv_path, link_to_path)` entry materialized into the runtime virtualenv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeVenvSymlink {
    /// Symlink destination relative to the virtualenv root.
    pub venv_path: String,
    /// Symlink target relative to the runfiles root.
    pub link_to_path: String,
}

/// Parsed command-line options for `native_bootstrap_embedder`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// Path to the input native stage-1 bootstrap template binary (CWD-relative or absolute).
    pub template: PathBuf,
    /// Path to write the embedded executable output binary (CWD-relative or absolute).
    pub output: PathBuf,
    /// Runfiles-root-relative path to the stage-2 Python bootstrap script.
    pub stage2_bootstrap_path: String,
    /// Path to the Python interpreter to invoke: runfiles-root-relative (typically the venv's
    /// `bin/python3`), absolute, or bare `PATH` executable name.
    pub python_binary: String,
    /// Underlying interpreter path that `python_binary` symlinks to for zip or runtime-venv
    /// execution: runfiles-root-relative, absolute, or bare `PATH` executable name.
    pub python_binary_actual: String,
    /// Whether the generated binary is a self-extracting zip archive.
    pub is_zipfile: bool,
    /// Whether stage-1 recreates the virtualenv at runtime.
    pub recreate_venv_at_runtime: bool,
    /// Whether stage-1 resolves `python_binary_actual` at runtime.
    pub resolve_python_binary_at_runtime: bool,
    /// Venv-root-relative `site-packages` directory path (e.g. `lib/python3.12/site-packages`).
    pub venv_rel_site_packages: String,
    /// Bazel workspace name.
    pub workspace_name: String,
    /// Bazel target label string.
    pub target: String,
    /// Arguments from the target's `interpreter_args` attribute passed to Python before stage-2.
    pub interpreter_args: Vec<String>,
    /// Explicit virtualenv symlinks (`<venv-root-relative>|<runfiles-root-relative>`) created at
    /// runtime.
    pub runtime_venv_symlinks: Vec<RuntimeVenvSymlink>,
}

/// Result of parsing the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    /// `-h` / `--help` was requested.
    Help,
    /// Embed bootstrap configuration with the given options.
    Run(Box<Options>),
}

#[derive(Default)]
struct ParseState {
    template: Option<PathBuf>,
    output: Option<PathBuf>,
    options: Options,
}

fn parse_no_nul_string(parser: &mut lexopt::Parser, flag: &str) -> Result<String> {
    let val = parser.value()?.string()?;
    if val.contains('\0') {
        bail!("--{flag} value must not contain NUL bytes");
    }
    Ok(val)
}

fn parse_bool_flag(parser: &mut lexopt::Parser, flag: &str) -> Result<bool> {
    let val = parser.value()?.string()?;
    match val.as_str() {
        "1" | "true" | "True" => Ok(true),
        "0" | "false" | "False" => Ok(false),
        _ => bail!("invalid --{flag} value (expected 0/1 or false/true): {val:?}"),
    }
}

fn parse_runtime_venv_symlink(parser: &mut lexopt::Parser) -> Result<RuntimeVenvSymlink> {
    let raw = parse_no_nul_string(parser, flags::RUNTIME_VENV_SYMLINK)?;
    let Some((venv_path, link_to_path)) = raw.split_once('|') else {
        bail!(
            "invalid --{} value (expected <venv_path>|<link_to_path>): {raw:?}",
            flags::RUNTIME_VENV_SYMLINK
        );
    };
    if venv_path.is_empty() || link_to_path.is_empty() {
        bail!(
            "invalid --{} value (expected non-empty <venv_path>|<link_to_path>): {raw:?}",
            flags::RUNTIME_VENV_SYMLINK
        );
    }
    Ok(RuntimeVenvSymlink {
        venv_path: venv_path.to_string(),
        link_to_path: link_to_path.to_string(),
    })
}

fn apply_flag(parser: &mut lexopt::Parser, flag: &str, state: &mut ParseState) -> Result<()> {
    match flag {
        flags::TEMPLATE => {
            state.template = Some(PathBuf::from(parser.value()?));
        }
        flags::OUTPUT => {
            state.output = Some(PathBuf::from(parser.value()?));
        }
        flags::STAGE2_BOOTSTRAP_PATH => {
            state.options.stage2_bootstrap_path =
                parse_no_nul_string(parser, flags::STAGE2_BOOTSTRAP_PATH)?;
        }
        flags::PYTHON_BINARY => {
            state.options.python_binary = parse_no_nul_string(parser, flags::PYTHON_BINARY)?;
        }
        flags::PYTHON_BINARY_ACTUAL => {
            state.options.python_binary_actual =
                parse_no_nul_string(parser, flags::PYTHON_BINARY_ACTUAL)?;
        }
        flags::IS_ZIPFILE => {
            state.options.is_zipfile = parse_bool_flag(parser, flags::IS_ZIPFILE)?;
        }
        flags::RECREATE_VENV_AT_RUNTIME => {
            state.options.recreate_venv_at_runtime =
                parse_bool_flag(parser, flags::RECREATE_VENV_AT_RUNTIME)?;
        }
        flags::RESOLVE_PYTHON_BINARY_AT_RUNTIME => {
            state.options.resolve_python_binary_at_runtime =
                parse_bool_flag(parser, flags::RESOLVE_PYTHON_BINARY_AT_RUNTIME)?;
        }
        flags::VENV_REL_SITE_PACKAGES => {
            state.options.venv_rel_site_packages =
                parse_no_nul_string(parser, flags::VENV_REL_SITE_PACKAGES)?;
        }
        flags::WORKSPACE_NAME => {
            state.options.workspace_name = parse_no_nul_string(parser, flags::WORKSPACE_NAME)?;
        }
        flags::TARGET => {
            state.options.target = parse_no_nul_string(parser, flags::TARGET)?;
        }
        flags::INTERPRETER_ARG => {
            state
                .options
                .interpreter_args
                .push(parse_no_nul_string(parser, flags::INTERPRETER_ARG)?);
        }
        flags::RUNTIME_VENV_SYMLINK => {
            state
                .options
                .runtime_venv_symlinks
                .push(parse_runtime_venv_symlink(parser)?);
        }
        _ => bail!("unexpected argument '--{flag}'"),
    }
    Ok(())
}

fn parse_flat_args<I>(args: I, state: &mut ParseState) -> Result<bool>
where
    I: IntoIterator<Item = OsString>,
{
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short(flags::HELP_SHORT) | Long(flags::HELP) => return Ok(true),
            Long(flag) => {
                let flag_owned = flag.to_string();
                apply_flag(&mut parser, &flag_owned, state)?;
            }
            _ => bail!("{}", arg.unexpected()),
        }
    }
    Ok(false)
}

/// Parses command-line arguments (excluding `argv[0]` and subcommand name) into [`Cli`].
///
/// Standalone `@<flagfile>` arguments are expanded with [`argfile`] (treating lines inside the
/// multiline response file as literal arguments so Bazel canonical labels such as `@@//pkg:bin`
/// after `--target` are not mistaken for nested response files).
///
/// # Errors
///
/// Returns an error if an `@flagfile` cannot be read, an unknown or malformed flag is passed, or
/// `--template` / `--output` is missing.
pub fn parse_args<I, T>(args: I) -> Result<Cli>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut state = ParseState::default();
    let mut parser = lexopt::Parser::from_args(args.into_iter().map(Into::into));
    while let Some(arg) = parser.next()? {
        match arg {
            Short(flags::HELP_SHORT) | Long(flags::HELP) => return Ok(Cli::Help),
            Long(flag) => {
                let flag_owned = flag.to_string();
                apply_flag(&mut parser, &flag_owned, &mut state)?;
            }
            Value(val) if val.as_encoded_bytes().starts_with(b"@") => {
                let expanded = argfile::expand_args_from(
                    std::iter::once(val),
                    |content, _prefix| {
                        content
                            .lines()
                            .map(|line| argfile::Argument::PassThrough(OsString::from(line)))
                            .collect()
                    },
                    argfile::PREFIX,
                )
                .context("reading @flagfile")?;
                if parse_flat_args(expanded, &mut state)? {
                    return Ok(Cli::Help);
                }
            }
            _ => bail!("{}", arg.unexpected()),
        }
    }

    let template = state
        .template
        .with_context(|| format!("missing required argument: --{}", flags::TEMPLATE))?;
    let output = state
        .output
        .with_context(|| format!("missing required argument: --{}", flags::OUTPUT))?;

    state.options.template = template;
    state.options.output = output;
    Ok(Cli::Run(Box::new(state.options)))
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), embeds target bootstrap
/// metadata into the template binary, and returns the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = match parse_args(args) {
        Ok(Cli::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Cli::Run(opts)) => *opts,
        Err(err) => {
            eprintln!("native_bootstrap_embedder: error: {err:#}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = embed_bootstrap_file(&options) {
        eprintln!("native_bootstrap_embedder: error: {err:#}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
