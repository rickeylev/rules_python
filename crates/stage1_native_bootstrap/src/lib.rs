//! Library for the stage-1 native bootstrap template and config embedder.

use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CFG_BUFFER_SIZE: usize = 4096;
pub const CFG_MAGIC_LEN: usize = 32;
pub const CFG_HEADER_LEN: usize = 36;
pub const EOF_TRAILER_MAGIC_LEN: usize = 24;
pub const EOF_TRAILER_LEN: usize = 32;

pub const CFG_MAGIC: [u8; CFG_MAGIC_LEN] = *b"\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\0";
pub const EOF_TRAILER_MAGIC: [u8; EOF_TRAILER_MAGIC_LEN] =
    *b"\x00RPY_NATIVE_EOF_CFG_V1\x00\x00";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BootstrapConfig {
    pub stage2_bootstrap: String,
    pub python_binary: String,
    pub python_binary_actual: String,
    pub is_zipfile: i32,
    pub recreate_venv_at_runtime: i32,
    pub resolve_python_binary_at_runtime: i32,
    pub venv_rel_site_packages: String,
    pub workspace_name: String,
    pub target: String,
    pub interpreter_args: Vec<String>,
    pub runtime_venv_symlinks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbedOptions {
    pub template_path: String,
    pub output_path: String,
    pub stage2_bootstrap: String,
    pub python_binary: String,
    pub python_binary_actual: String,
    pub is_zipfile: String,
    pub recreate_venv_at_runtime: String,
    pub resolve_python_binary_at_runtime: String,
    pub venv_rel_site_packages: String,
    pub workspace_name: String,
    pub target: String,
    pub use_eof_trailer: bool,
    pub interpreter_args: Vec<String>,
    pub runtime_venv_symlinks: Vec<String>,
}

impl Default for EmbedOptions {
    fn default() -> Self {
        Self {
            template_path: String::new(),
            output_path: String::new(),
            stage2_bootstrap: String::new(),
            python_binary: String::new(),
            python_binary_actual: String::new(),
            is_zipfile: "0".to_string(),
            recreate_venv_at_runtime: "0".to_string(),
            resolve_python_binary_at_runtime: "0".to_string(),
            venv_rel_site_packages: String::new(),
            workspace_name: String::new(),
            target: String::new(),
            use_eof_trailer: false,
            interpreter_args: Vec::new(),
            runtime_venv_symlinks: Vec::new(),
        }
    }
}

fn push_kv(buf: &mut Vec<u8>, key: &str, val: &str) {
    buf.extend_from_slice(key.as_bytes());
    buf.push(b'=');
    buf.extend_from_slice(val.as_bytes());
    buf.push(0);
}

/// Serializes `EmbedOptions` into a NUL-delimited `key=value` byte payload.
pub fn serialize_cfg_payload(opts: &EmbedOptions) -> Vec<u8> {
    let mut payload = Vec::new();
    push_kv(&mut payload, "stage2_bootstrap", &opts.stage2_bootstrap);
    push_kv(&mut payload, "python_binary", &opts.python_binary);
    push_kv(&mut payload, "python_binary_actual", &opts.python_binary_actual);
    push_kv(&mut payload, "is_zipfile", &opts.is_zipfile);
    push_kv(
        &mut payload,
        "recreate_venv_at_runtime",
        &opts.recreate_venv_at_runtime,
    );
    push_kv(
        &mut payload,
        "resolve_python_binary_at_runtime",
        &opts.resolve_python_binary_at_runtime,
    );
    push_kv(
        &mut payload,
        "venv_rel_site_packages",
        &opts.venv_rel_site_packages,
    );
    push_kv(&mut payload, "workspace_name", &opts.workspace_name);
    push_kv(&mut payload, "target", &opts.target);
    for arg in &opts.interpreter_args {
        push_kv(&mut payload, "interpreter_arg", arg);
    }
    for sym in &opts.runtime_venv_symlinks {
        push_kv(&mut payload, "runtime_venv_symlink", sym);
    }
    payload.push(0);
    payload
}

/// Parses a NUL-delimited `key=value` byte slice into `BootstrapConfig`.
pub fn parse_cfg_payload(payload: &[u8], cfg: &mut BootstrapConfig) {
    let mut pos = 0;
    while pos < payload.len() {
        let rem = &payload[pos..];
        let Some(nul_rel) = rem.iter().position(|&b| b == 0) else {
            break;
        };
        let entry = &rem[..nul_rel];
        pos += nul_rel + 1;
        if entry.is_empty() {
            break;
        }
        let Some(eq_idx) = entry.iter().position(|&b| b == b'=') else {
            continue;
        };
        let key = &entry[..eq_idx];
        let val = String::from_utf8_lossy(&entry[eq_idx + 1..]).into_owned();
        match key {
            b"stage2_bootstrap" => cfg.stage2_bootstrap = val,
            b"python_binary" => cfg.python_binary = val,
            b"python_binary_actual" => cfg.python_binary_actual = val,
            b"is_zipfile" => cfg.is_zipfile = val.parse().unwrap_or(0),
            b"recreate_venv_at_runtime" => {
                cfg.recreate_venv_at_runtime = val.parse().unwrap_or(0)
            }
            b"resolve_python_binary_at_runtime" => {
                cfg.resolve_python_binary_at_runtime = val.parse().unwrap_or(0)
            }
            b"venv_rel_site_packages" => cfg.venv_rel_site_packages = val,
            b"workspace_name" => cfg.workspace_name = val,
            b"target" => cfg.target = val,
            b"interpreter_arg" => {
                if !val.is_empty() {
                    cfg.interpreter_args.push(val);
                }
            }
            b"runtime_venv_symlink" => {
                if !val.is_empty() {
                    cfg.runtime_venv_symlinks.push(val);
                }
            }
            _ => {}
        }
    }
}

/// Expands `@param_file` arguments into a flat argument vector.
fn expand_param_files(args: &[String]) -> Result<Vec<String>, String> {
    let mut expanded = Vec::with_capacity(args.len());
    if let Some(first) = args.first() {
        expanded.push(first.clone());
    }
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg.starts_with("--") {
            expanded.push(arg.clone());
            if i + 1 < args.len() {
                expanded.push(args[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
        } else if let Some(param_path) = arg.strip_prefix('@') {
            let content = fs::read_to_string(param_path)
                .map_err(|e| format!("failed to read param file {param_path}: {e}"))?;
            for line in content.lines() {
                expanded.push(line.to_string());
            }
            i += 1;
        } else {
            expanded.push(arg.clone());
            i += 1;
        }
    }
    Ok(expanded)
}

/// Parses CLI arguments for `stage1_native_bootstrap_embedder`.
pub fn parse_embed_args(args: &[String]) -> Result<EmbedOptions, String> {
    let expanded = expand_param_files(args)?;
    let mut opts = EmbedOptions::default();
    let mut i = 1;
    while i < expanded.len() {
        let flag = expanded[i].as_str();
        if i + 1 >= expanded.len() {
            return Err(format!("unknown or incomplete arg: {flag}"));
        }
        let val = expanded[i + 1].clone();
        i += 2;
        match flag {
            "--template" => opts.template_path = val,
            "--output" => opts.output_path = val,
            "--stage2_bootstrap" => opts.stage2_bootstrap = val,
            "--python_binary" => opts.python_binary = val,
            "--python_binary_actual" => opts.python_binary_actual = val,
            "--is_zipfile" => opts.is_zipfile = val,
            "--recreate_venv_at_runtime" => opts.recreate_venv_at_runtime = val,
            "--resolve_python_binary_at_runtime" => {
                opts.resolve_python_binary_at_runtime = val
            }
            "--venv_rel_site_packages" => opts.venv_rel_site_packages = val,
            "--workspace_name" => opts.workspace_name = val,
            "--target" => opts.target = val,
            "--eof_trailer" => opts.use_eof_trailer = val == "1" || val == "true",
            "--interpreter_arg" => opts.interpreter_args.push(val),
            "--runtime_venv_symlink" => opts.runtime_venv_symlinks.push(val),
            _ => return Err(format!("unknown or incomplete arg: {flag}")),
        }
    }
    if opts.template_path.is_empty() || opts.output_path.is_empty() {
        return Err("--template and --output are required".to_string());
    }
    Ok(opts)
}

pub const CFG_FOOTER_ZERO_LEN: usize = 16;

/// Returns whether `tpl_buf` contains the 32-byte `CFG_MAGIC` sentinel.
pub fn has_magic_sentinel(tpl_buf: &[u8]) -> bool {
    tpl_buf
        .windows(CFG_MAGIC_LEN)
        .any(|window| window == CFG_MAGIC)
}

/// Finds the byte offset of the 4 KiB `CFG_MAGIC` buffer in `tpl_buf`.
pub fn find_magic_offset(tpl_buf: &[u8]) -> Option<usize> {
    if tpl_buf.len() < CFG_BUFFER_SIZE {
        return None;
    }
    let max_start = tpl_buf.len() - CFG_BUFFER_SIZE;
    (0..=max_start).find(|&i| {
        tpl_buf[i..i + CFG_MAGIC_LEN] == CFG_MAGIC
            && tpl_buf[i + CFG_BUFFER_SIZE - CFG_FOOTER_ZERO_LEN..i + CFG_BUFFER_SIZE]
                == [0u8; CFG_FOOTER_ZERO_LEN]
    })
}

/// Embeds `opts` into `tpl_buf`.
///
/// - On Unix (default), patches the 4 KiB `.data` sentinel buffer in-place and
///   returns an error if the serialized payload exceeds `4,044` bytes.
/// - On Windows (`opts.use_eof_trailer` or a PE `MZ` binary), always appends
///   the variable-sized EOF trailer (`[payload][payload_len: u64_le][EOF_TRAILER_MAGIC]`).
pub fn embed_config_into_binary(
    tpl_buf: &[u8],
    opts: &EmbedOptions,
) -> Result<Vec<u8>, String> {
    let payload = serialize_cfg_payload(opts);
    let use_eof = opts.use_eof_trailer || tpl_buf.starts_with(b"MZ");

    if use_eof {
        if !has_magic_sentinel(tpl_buf) {
            return Err(
                "template binary is missing native bootstrap magic sentinel \
                 (\\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\\0); ensure bootstrap_template \
                 is a native bootstrap binary when --bootstrap_impl=native"
                    .to_string(),
            );
        }
        let mut out = Vec::with_capacity(tpl_buf.len() + payload.len() + EOF_TRAILER_LEN);
        out.extend_from_slice(tpl_buf);
        out.extend_from_slice(&payload);
        out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        out.extend_from_slice(&EOF_TRAILER_MAGIC);
        return Ok(out);
    }

    let Some(magic_offset) = find_magic_offset(tpl_buf) else {
        return Err(
            "template binary is missing native bootstrap magic sentinel \
             (\\x89RPY_NATIVE_BOOTSTRAP_CFG_V1_00\\0); ensure bootstrap_template \
             is a native bootstrap binary when --bootstrap_impl=native"
                .to_string(),
        );
    };

    let max_inplace_len = CFG_BUFFER_SIZE - CFG_HEADER_LEN - CFG_FOOTER_ZERO_LEN;
    if payload.len() > max_inplace_len {
        return Err(format!(
            "bootstrap config payload ({} bytes) exceeds 4 KiB in-place buffer capacity ({max_inplace_len} bytes)",
            payload.len()
        ));
    }

    let mut out = tpl_buf.to_vec();
    let cfg_region = &mut out[magic_offset..magic_offset + CFG_BUFFER_SIZE];
    cfg_region[CFG_MAGIC_LEN..].fill(0);
    let plen = (payload.len() as u32).to_le_bytes();
    cfg_region[32..36].copy_from_slice(&plen);
    cfg_region[CFG_HEADER_LEN..CFG_HEADER_LEN + payload.len()]
        .copy_from_slice(&payload);
    Ok(out)
}

fn set_executable_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Reads `opts.template_path`, embeds `opts`, and writes the executable to
/// `opts.output_path`.
pub fn embed_bootstrap_file(opts: &EmbedOptions) -> io::Result<()> {
    let tpl_buf = fs::read(&opts.template_path)?;
    let out_buf = embed_config_into_binary(&tpl_buf, opts)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let output_path = Path::new(&opts.output_path);
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut f = File::create(output_path)?;
    f.write_all(&out_buf)?;
    f.flush()?;
    drop(f);
    set_executable_permissions(output_path)?;
    Ok(())
}

/// Entrypoint helper for `stage1_native_bootstrap_embedder`.
pub fn run_embedder(args: Vec<String>) -> Result<(), String> {
    let opts = parse_embed_args(&args)?;
    embed_bootstrap_file(&opts).map_err(|e| format!("{}: {}", opts.output_path, e))
}

/// Parses the EOF trailer (`[payload][payload_len: u64_le][EOF_TRAILER_MAGIC]`)
/// from the end of `file_buf`.
pub fn parse_eof_trailer_from_bytes(
    file_buf: &[u8],
    cfg: &mut BootstrapConfig,
) -> Result<(), String> {
    if file_buf.len() < EOF_TRAILER_LEN {
        return Err("unconfigured native bootstrap template binary".to_string());
    }
    let magic_start = file_buf.len() - EOF_TRAILER_MAGIC_LEN;
    if file_buf[magic_start..] != EOF_TRAILER_MAGIC {
        return Err("unconfigured native bootstrap template binary".to_string());
    }
    let len_start = file_buf.len() - EOF_TRAILER_LEN;
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&file_buf[len_start..magic_start]);
    let len = u64::from_le_bytes(len_bytes) as usize;
    if len == 0 || len > len_start {
        return Err("unconfigured native bootstrap template binary".to_string());
    }
    let payload_start = len_start - len;
    parse_cfg_payload(&file_buf[payload_start..len_start], cfg);
    Ok(())
}

/// Seeks to the end of `self_exe` and reads the Windows EOF trailer payload in
/// $O(1)$ without loading the executable image into memory.
pub fn load_eof_trailer_config(self_exe: &str) -> Result<BootstrapConfig, String> {
    let mut f = File::open(self_exe)
        .map_err(|e| format!("failed to open self executable for EOF trailer: {e}"))?;
    let file_len = f
        .seek(SeekFrom::End(0))
        .map_err(|e| format!("failed to seek to end of executable: {e}"))?;
    if file_len < EOF_TRAILER_LEN as u64 {
        return Err("unconfigured native bootstrap template binary".to_string());
    }
    f.seek(SeekFrom::Start(file_len - EOF_TRAILER_LEN as u64))
        .map_err(|e| format!("failed to seek to EOF trailer footer: {e}"))?;
    let mut footer = [0u8; EOF_TRAILER_LEN];
    f.read_exact(&mut footer)
        .map_err(|e| format!("failed to read EOF trailer footer: {e}"))?;
    if footer[8..] != EOF_TRAILER_MAGIC {
        return Err("unconfigured native bootstrap template binary".to_string());
    }
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&footer[..8]);
    let payload_len = u64::from_le_bytes(len_bytes);
    let trailer_start = file_len - EOF_TRAILER_LEN as u64;
    if payload_len == 0 || payload_len > trailer_start {
        return Err("unconfigured native bootstrap template binary".to_string());
    }
    f.seek(SeekFrom::Start(trailer_start - payload_len))
        .map_err(|e| format!("failed to seek to EOF trailer payload: {e}"))?;
    let mut payload = vec![0u8; payload_len as usize];
    f.read_exact(&mut payload)
        .map_err(|e| format!("failed to read EOF trailer payload: {e}"))?;
    let mut cfg = BootstrapConfig::default();
    parse_cfg_payload(&payload, &mut cfg);
    Ok(cfg)
}

pub fn join_path(a: &str, b: &str) -> String {
    if a.is_empty() {
        return b.to_string();
    }
    if b.is_empty() {
        return a.to_string();
    }
    if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

pub fn join_path3(a: &str, b: &str, c: &str) -> String {
    let ab = join_path(a, b);
    join_path(&ab, c)
}

pub fn dirname_str(path: &str) -> String {
    match path.rfind('/') {
        None => ".".to_string(),
        Some(0) => "/".to_string(),
        Some(idx) => path[..idx].to_string(),
    }
}

pub fn basename_str(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[idx + 1..],
        None => path,
    }
}

/// Strips the last two path components (`dirname(dirname(path))`).
pub fn strip_last_two_components(path: &str) -> String {
    let first = dirname_str(path);
    dirname_str(&first)
}

/// Checks if `path` contains `".runfiles/"` and returns the prefix ending at
/// `".runfiles"` (matching up to the last `".runfiles/"` segment).
pub fn extract_runfiles_prefix(path: &str) -> Option<String> {
    let needle = ".runfiles/";
    let idx = path.rfind(needle)?;
    Some(path[..idx + ".runfiles".len()].to_string())
}

fn is_symlink_path(path: &str) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

fn is_executable_file(path: &str) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    if meta.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn find_in_path(name: &str, path_env: Option<&str>) -> Option<String> {
    if name.contains('/') {
        return Some(name.to_string());
    }
    let path_str = path_env?;
    for dir in path_str.split(':') {
        let effective_dir = if dir.is_empty() { "." } else { dir };
        let candidate = join_path(effective_dir, name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn current_dir_string() -> Result<String, String> {
    env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| format!("getcwd: {e}"))
}

pub fn resolve_self_executable(argv0: &str) -> Result<String, String> {
    if argv0.starts_with('/') && Path::new(argv0).exists() {
        return Ok(argv0.to_string());
    }
    if argv0.contains('/') {
        if let Ok(cwd) = current_dir_string() {
            let abs_path = join_path(&cwd, argv0);
            if Path::new(&abs_path).exists() {
                return Ok(abs_path);
            }
        }
    }
    if let Ok(exe) = env::current_exe() {
        if exe.exists() {
            return Ok(exe.to_string_lossy().into_owned());
        }
    }
    let path_env = env::var("PATH").ok();
    if let Some(found) = find_in_path(argv0, path_env.as_deref()) {
        return Ok(found);
    }
    Err("unable to determine path to self executable".to_string())
}

/// Discovers the runfiles root directory using `RUNFILES_DIR`,
/// `RUNFILES_MANIFEST_FILE`, or symlink traversal starting from `argv0`.
pub fn find_runfiles_root_with_env(
    argv0: &str,
    stage2_bootstrap: &str,
    runfiles_dir_env: Option<&str>,
    runfiles_manifest_env: Option<&str>,
    cwd: &str,
) -> Result<String, String> {
    let mut maybe_root: Option<String> = None;
    if let Some(dir) = runfiles_dir_env.filter(|s| !s.is_empty()) {
        maybe_root = Some(dir.to_string());
    } else if let Some(manifest) = runfiles_manifest_env.filter(|s| !s.is_empty()) {
        if let Some(prefix) = manifest.strip_suffix(".runfiles_manifest") {
            maybe_root = Some(format!("{prefix}.runfiles"));
        } else if let Some(prefix) = manifest.strip_suffix(".runfiles/MANIFEST") {
            maybe_root = Some(format!("{prefix}.runfiles"));
        }
    }

    if let Some(root) = maybe_root {
        let stage2_path = join_path(&root, stage2_bootstrap);
        if Path::new(&stage2_path).exists() {
            return Ok(root);
        }
    }

    let mut stub_filename = if argv0.starts_with('/') {
        argv0.to_string()
    } else {
        join_path(cwd, argv0)
    };

    for _hop in 0..64 {
        let module_space = format!("{stub_filename}.runfiles");
        if Path::new(&module_space).is_dir() {
            return Ok(module_space);
        }
        if let Some(prefix) = extract_runfiles_prefix(&stub_filename) {
            return Ok(prefix);
        }
        if !is_symlink_path(&stub_filename) {
            break;
        }
        let Ok(link_target_path) = fs::read_link(&stub_filename) else {
            break;
        };
        let link_target = link_target_path.to_string_lossy().into_owned();
        stub_filename = if link_target.starts_with('/') {
            link_target
        } else {
            let stub_dir = dirname_str(&stub_filename);
            let rel_joined = join_path(&stub_dir, &link_target);
            if fs::symlink_metadata(&rel_joined).is_ok()
                || fs::symlink_metadata(&link_target).is_err()
            {
                rel_joined
            } else {
                link_target
            }
        };
    }

    Err(format!(
        "Cannot find .runfiles directory for {argv0} (last checked {stub_filename})"
    ))
}

pub fn find_runfiles_root(argv0: &str, stage2_bootstrap: &str) -> Result<String, String> {
    let runfiles_dir_env = env::var("RUNFILES_DIR").ok();
    let runfiles_manifest_env = env::var("RUNFILES_MANIFEST_FILE").ok();
    let cwd = current_dir_string()?;
    find_runfiles_root_with_env(
        argv0,
        stage2_bootstrap,
        runfiles_dir_env.as_deref(),
        runfiles_manifest_env.as_deref(),
        &cwd,
    )
}

pub fn resolve_actual_interpreter(
    python_binary_actual: &str,
    runfiles_dir: &str,
) -> Result<String, String> {
    if python_binary_actual.starts_with('/') {
        return Ok(python_binary_actual.to_string());
    }
    if python_binary_actual.contains('/') {
        return Ok(join_path(runfiles_dir, python_binary_actual));
    }
    let path_env = env::var("PATH").ok();
    find_in_path(python_binary_actual, path_env.as_deref()).ok_or_else(|| {
        format!("Python interpreter not found on PATH: {python_binary_actual}")
    })
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Creates a unique temporary directory under `base_tmp` starting with `prefix`.
pub fn create_temp_dir(base_tmp: &str, prefix: &str) -> io::Result<String> {
    fs::create_dir_all(base_tmp)?;
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    for attempt in 0..10000u64 {
        let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir_name = format!(
            "{prefix}{pid}_{}_{attempt}",
            nanos.wrapping_add(seq) & 0xFFFF_FFFF
        );
        let candidate = join_path(base_tmp, &dir_name);
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to create unique temporary directory",
    ))
}

fn create_symlink_force(src: &str, dst: &str) -> io::Result<()> {
    let _ = fs::remove_file(dst);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst)
    }
    #[cfg(not(unix))]
    {
        fs::copy(src, dst).map(|_| ())
    }
}

fn extract_zip_archive(self_exe: &str, zip_dir: &str, verbose: bool) -> Result<(), String> {
    let mut cmd = Command::new("unzip");
    cmd.arg("-q").arg("-d").arg(zip_dir).arg(self_exe);
    if !verbose {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let _ = cmd
        .status()
        .map_err(|e| format!("failed to execute unzip: {e}"))?;
    Ok(())
}

fn resolve_python_binary_via_subprocess(
    python_cmd: &str,
    venv: &str,
) -> Result<(String, String), String> {
    let output = Command::new(python_cmd)
        .arg("-I")
        .arg("-c")
        .arg(
            "import sys, site; print(sys.executable); \
             print(site.getsitepackages([sys.argv[1]])[-1])",
        )
        .arg(venv)
        .stdout(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to run python runtime resolution: {e}"))?;

    if !output.status.success() {
        return Err("failed to resolve python binary at runtime".to_string());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let exe = lines
        .next()
        .ok_or_else(|| "unexpected output from python runtime resolution".to_string())?
        .trim_end_matches('\r')
        .to_string();
    let site_pkgs = lines
        .next()
        .ok_or_else(|| "unexpected output from python runtime resolution".to_string())?
        .trim_end_matches('\r')
        .to_string();
    Ok((exe, site_pkgs))
}

fn is_nonempty_env(name: &str) -> bool {
    env::var_os(name).map(|v| !v.is_empty()).unwrap_or(false)
}

#[cfg(unix)]
const SIGHUP: i32 = 1;
#[cfg(unix)]
const SIGINT: i32 = 2;
#[cfg(unix)]
const SIGQUIT: i32 = 3;
#[cfg(unix)]
const SIGTERM: i32 = 15;
#[cfg(unix)]
const SIG_IGN: usize = 1;

static G_CHILD_PID: AtomicI32 = AtomicI32::new(0);
#[cfg(unix)]
static G_RECEIVED_SIG: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
    fn signal(sig: i32, handler: usize) -> usize;
}

#[cfg(unix)]
extern "C" fn forward_signal_handler(sig: i32) {
    G_RECEIVED_SIG.store(sig, Ordering::SeqCst);
    let child = G_CHILD_PID.load(Ordering::SeqCst);
    if child > 0 {
        unsafe {
            kill(child, sig);
        }
    }
}

/// Runs stage-1 bootstrap execution using a parsed `BootstrapConfig`.
pub fn run_stage1_with_config(cfg: BootstrapConfig, args: &[String]) -> i32 {
    let argv0 = args
        .first()
        .map(|s| s.as_str())
        .unwrap_or("stage1_native_bootstrap");
    let verbose = is_nonempty_env("RULES_PYTHON_BOOTSTRAP_VERBOSE");

    let mut zip_dir_to_clean: Option<String> = None;
    let mut venv_dir_to_clean: Option<String> = None;

    let runfiles_dir = if cfg.is_zipfile == 1 {
        let self_exe = match resolve_self_executable(argv0) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("stage1_native_bootstrap: {e}");
                return 1;
            }
        };
        let extract_root = env::var("RULES_PYTHON_EXTRACT_ROOT").ok();
        let tmpdir_env = env::var("TMPDIR").ok();
        let base_tmp = extract_root
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| tmpdir_env.as_deref().filter(|s| !s.is_empty()))
            .unwrap_or("/tmp");

        let zip_dir = match create_temp_dir(base_tmp, "rules_python_zip_tmp.") {
            Ok(d) => d,
            Err(e) => {
                eprintln!("stage1_native_bootstrap: create_temp_dir for zip_dir: {e}");
                return 1;
            }
        };
        if verbose {
            eprintln!("RULES_PYTHON_BOOTSTRAP_VERBOSE: Extracting {self_exe} to {zip_dir}");
        } else {
            zip_dir_to_clean = Some(zip_dir.clone());
        }

        if let Err(e) = extract_zip_archive(&self_exe, &zip_dir, verbose) {
            if let Some(ref d) = zip_dir_to_clean {
                let _ = fs::remove_dir_all(d);
            }
            eprintln!("stage1_native_bootstrap: {e}");
            return 1;
        }
        let rf_dir = join_path(&zip_dir, "runfiles");
        if !Path::new(&rf_dir).is_dir() {
            if let Some(ref d) = zip_dir_to_clean {
                let _ = fs::remove_dir_all(d);
            }
            eprintln!("Zip directory does not appear to contain runfiles");
            return 1;
        }

        let python_exe_in_zip = join_path(&rf_dir, &cfg.python_binary);
        let symlink_to = match resolve_actual_interpreter(&cfg.python_binary_actual, &rf_dir) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("stage1_native_bootstrap: {e}");
                return 1;
            }
        };
        let python_exe_dir = dirname_str(&python_exe_in_zip);
        let _ = fs::create_dir_all(&python_exe_dir);
        if let Err(e) = create_symlink_force(&symlink_to, &python_exe_in_zip) {
            eprintln!("stage1_native_bootstrap: symlink python_exe in zip: {e}");
            return 1;
        }

        if let Ok(meta) = fs::metadata(&self_exe) {
            env::set_var("STAGE2_BOOTSTRAP_OFFSET", meta.len().to_string());
        }
        rf_dir
    } else {
        match find_runfiles_root(argv0, &cfg.stage2_bootstrap) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        }
    };

    if is_nonempty_env("RULES_PYTHON_TESTING_TELL_RUNFILES_ROOT") {
        env::set_var("RULES_PYTHON_TESTING_RUNFILES_ROOT", &runfiles_dir);
    }

    let python_exe = if cfg.recreate_venv_at_runtime == 1 {
        let build_venv = strip_last_two_components(&cfg.python_binary);
        let extract_root = env::var("RULES_PYTHON_EXTRACT_ROOT").ok();
        let venv = if let Some(root) = extract_root.as_deref().filter(|s| !s.is_empty()) {
            join_path(root, &build_venv)
        } else {
            let tmpdir_env = env::var("TMPDIR").ok();
            let base_tmp = tmpdir_env
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("/tmp");
            let d = match create_temp_dir(base_tmp, "rules_python_venv_tmp.") {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("stage1_native_bootstrap: create_temp_dir for venv: {e}");
                    return 1;
                }
            };
            if !verbose {
                venv_dir_to_clean = Some(d.clone());
            }
            d
        };

        let symlink_to = match resolve_actual_interpreter(&cfg.python_binary_actual, &runfiles_dir)
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("stage1_native_bootstrap: {e}");
                return 1;
            }
        };
        let venv_bin = join_path(&venv, "bin");
        if let Err(e) = fs::create_dir_all(&venv_bin) {
            eprintln!("stage1_native_bootstrap: mkdir {venv_bin}: {e}");
            return 1;
        }
        let py_exe = join_path(&venv_bin, basename_str(&cfg.python_binary_actual));

        if cfg.resolve_python_binary_at_runtime == 1 {
            let (resolved_exe, venv_site_packages) =
                match resolve_python_binary_via_subprocess(&symlink_to, &venv) {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("stage1_native_bootstrap: {e}");
                        return 1;
                    }
                };
            if let Err(e) = create_symlink_force(&resolved_exe, &py_exe) {
                eprintln!("stage1_native_bootstrap: symlink resolved python_exe: {e}");
                return 1;
            }
            let site_parent = dirname_str(&venv_site_packages);
            let _ = fs::create_dir_all(&site_parent);
            let build_site_packages =
                join_path3(&runfiles_dir, &build_venv, &cfg.venv_rel_site_packages);
            if let Err(e) = create_symlink_force(&build_site_packages, &venv_site_packages) {
                eprintln!("stage1_native_bootstrap: symlink venv_site_packages: {e}");
                return 1;
            }
        } else {
            if let Err(e) = create_symlink_force(&symlink_to, &py_exe) {
                eprintln!("stage1_native_bootstrap: symlink python_exe in venv: {e}");
                return 1;
            }
            let build_lib = join_path3(&runfiles_dir, &build_venv, "lib");
            let venv_lib = join_path(&venv, "lib");
            if let Err(e) = create_symlink_force(&build_lib, &venv_lib) {
                eprintln!("stage1_native_bootstrap: symlink venv lib: {e}");
                return 1;
            }
        }

        let build_pyvenv_cfg = join_path3(&runfiles_dir, &build_venv, "pyvenv.cfg");
        let venv_pyvenv_cfg = join_path(&venv, "pyvenv.cfg");
        if let Err(e) = create_symlink_force(&build_pyvenv_cfg, &venv_pyvenv_cfg) {
            eprintln!("stage1_native_bootstrap: symlink pyvenv.cfg: {e}");
            return 1;
        }

        for entry in &cfg.runtime_venv_symlinks {
            let Some(pipe_idx) = entry.find('|') else {
                continue;
            };
            let venv_rel_path = &entry[..pipe_idx];
            let runfiles_rel_target = &entry[pipe_idx + 1..];
            let dest_link = join_path(&venv, venv_rel_path);
            let dest_parent = dirname_str(&dest_link);
            let _ = fs::create_dir_all(&dest_parent);
            let src_target = join_path(&runfiles_dir, runfiles_rel_target);
            if let Err(e) = create_symlink_force(&src_target, &dest_link) {
                eprintln!("stage1_native_bootstrap: symlink runtime_venv_symlink: {e}");
                return 1;
            }
        }

        py_exe
    } else {
        join_path(&runfiles_dir, &cfg.python_binary)
    };

    env::set_var("RUNFILES_DIR", &runfiles_dir);
    env::set_var("JAVA_RUNFILES", &runfiles_dir);

    if is_nonempty_env("RUNFILES_MANIFEST_FILE") {
        let manifest_in_dir = join_path(&runfiles_dir, "MANIFEST");
        if Path::new(&manifest_in_dir).exists() {
            env::set_var("RUNFILES_MANIFEST_FILE", &manifest_in_dir);
        } else {
            let dot_manifest = format!("{runfiles_dir}_manifest");
            if Path::new(&dot_manifest).exists() {
                env::set_var("RUNFILES_MANIFEST_FILE", &dot_manifest);
            } else {
                env::set_var("RUNFILES_MANIFEST_FILE", "");
            }
        }
    }

    let safepath = match env::var("PYTHONSAFEPATH") {
        Ok(val) => val,
        Err(_) => {
            env::set_var("PYTHONSAFEPATH", "1");
            "1".to_string()
        }
    };

    let mut child_args: Vec<String> = Vec::new();
    if !safepath.is_empty() {
        child_args.push("-I".to_string());
    }
    for arg in &cfg.interpreter_args {
        child_args.push(arg.clone());
    }
    if let Ok(extra) = env::var("RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS") {
        if !extra.is_empty() {
            for tok in extra.split_whitespace() {
                child_args.push(tok.to_string());
            }
            env::remove_var("RULES_PYTHON_ADDITIONAL_INTERPRETER_ARGS");
        }
    }
    let stage2_abs = join_path(&runfiles_dir, &cfg.stage2_bootstrap);
    child_args.push(stage2_abs);
    for user_arg in args.iter().skip(1) {
        child_args.push(user_arg.clone());
    }

    if zip_dir_to_clean.is_none() && venv_dir_to_clean.is_none() {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let err = Command::new(&python_exe).args(&child_args).exec();
            eprintln!("stage1_native_bootstrap: {python_exe}: {err}");
            return 1;
        }
    }

    #[cfg(unix)]
    {
        for sig in [SIGHUP, SIGINT, SIGQUIT, SIGTERM] {
            unsafe {
                let prev = signal(sig, forward_signal_handler as *const () as usize);
                if prev == SIG_IGN {
                    signal(sig, SIG_IGN);
                }
            }
        }
    }

    let mut child = match Command::new(&python_exe).args(&child_args).spawn() {
        Ok(c) => c,
        Err(e) => {
            if let Some(ref d) = venv_dir_to_clean {
                let _ = fs::remove_dir_all(d);
            }
            if let Some(ref d) = zip_dir_to_clean {
                let _ = fs::remove_dir_all(d);
            }
            eprintln!("stage1_native_bootstrap: spawn({python_exe}): {e}");
            return 127;
        }
    };

    let child_pid = child.id() as i32;
    G_CHILD_PID.store(child_pid, Ordering::SeqCst);
    #[cfg(unix)]
    {
        let pending_sig = G_RECEIVED_SIG.load(Ordering::SeqCst);
        if pending_sig != 0 {
            unsafe {
                kill(child_pid, pending_sig);
            }
        }
    }

    let wait_res = child.wait();

    if let Some(ref d) = venv_dir_to_clean {
        let _ = fs::remove_dir_all(d);
    }
    if let Some(ref d) = zip_dir_to_clean {
        let _ = fs::remove_dir_all(d);
    }

    match wait_res {
        Ok(status) => {
            if let Some(code) = status.code() {
                return code;
            }
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if let Some(sig) = status.signal() {
                    return 128 + sig;
                }
            }
            1
        }
        Err(_) => 1,
    }
}
