use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// "!ARGS\0\0\0" followed by 16-byte raw UUID "6ca2171c-7b91-4fe8-87b3-ce5912466b47" (24 bytes total).
// Total footer size at EOF = 8 bytes length + 24 bytes magic = 32 bytes (8-byte aligned).
pub const MAGIC: &[u8; 24] = &[
    b'!', b'A', b'R', b'G', b'S', 0, 0, 0,
    0x6c, 0xa2, 0x17, 0x1c, 0x7b, 0x91, 0x4f, 0xe8,
    0x87, 0xb3, 0xce, 0x59, 0x12, 0x46, 0x6b, 0x47,
];

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct EmbeddedPayload {
    pub src_out: Option<String>,
    pub out: Option<String>,
    pub args: Vec<String>,
}

impl EmbeddedPayload {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut payload = EmbeddedPayload::default();
        let mut start = 0;
        for i in 0..bytes.len() {
            if bytes[i] == 0 {
                let entry = &bytes[start..i];
                start = i + 1;
                if entry.is_empty() {
                    continue;
                }
                let entry_str = std::str::from_utf8(entry)
                    .map_err(|e| format!("Invalid UTF-8 in payload: {}", e))?;
                if let Some((key, value)) = entry_str.split_once('=') {
                    match key {
                        "src_out" => {
                            payload.src_out = Some(value.to_string());
                        }
                        "out" => {
                            payload.out = Some(value.to_string());
                        }
                        "arg" | "args" => {
                            payload.args.push(value.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(payload)
    }
}

pub fn read_trailer(path: &Path) -> io::Result<Option<EmbeddedPayload>> {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let len = file.metadata()?.len();
    let footer_len = 8 + MAGIC.len();
    if len < footer_len as u64 {
        return Ok(None);
    }

    file.seek(SeekFrom::End(-(footer_len as i64)))?;
    let mut footer = vec![0u8; footer_len];
    file.read_exact(&mut footer)?;

    if &footer[8..] != MAGIC {
        return Ok(None);
    }

    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&footer[0..8]);
    let payload_len = u64::from_le_bytes(len_bytes) as usize;
    if len < (footer_len as u64 + payload_len as u64) {
        return Ok(None);
    }

    file.seek(SeekFrom::End(-(footer_len as i64 + payload_len as i64)))?;
    let mut payload_bytes = vec![0u8; payload_len];
    file.read_exact(&mut payload_bytes)?;

    let payload = EmbeddedPayload::from_bytes(&payload_bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    Ok(Some(payload))
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct Config {
    pub src_out: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub command: Vec<OsString>,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let mut config = Config::default();

        // 1. First, check if the executable itself has an embedded trailer.
        if let Ok(current_exe) = env::current_exe() {
            if let Ok(Some(payload)) = read_trailer(&current_exe) {
                if let Some(s) = payload.src_out {
                    if !s.is_empty() {
                        config.src_out = Some(PathBuf::from(s));
                    }
                }
                if let Some(o) = payload.out {
                    if !o.is_empty() {
                        config.out = Some(PathBuf::from(o));
                    }
                }
                config.command = payload.args.into_iter().map(OsString::from).collect();
            }
        }

        // 2. Parse CLI arguments
        let args: Vec<OsString> = env::args_os().skip(1).collect();
        config.merge_cli_args(args)?;

        // 3. Fall back to environment variables
        if config.src_out.is_none() {
            if let Some(val) = env::var_os("UV_LOCK_SRC_OUT") {
                if !val.is_empty() {
                    config.src_out = Some(PathBuf::from(val));
                }
            }
        }
        if config.out.is_none() {
            if let Some(val) = env::var_os("UV_LOCK_OUT") {
                if !val.is_empty() {
                    config.out = Some(PathBuf::from(val));
                }
            }
        }

        Ok(config)
    }

    pub fn merge_cli_args<I>(&mut self, args: I) -> Result<(), String>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut iter = args.into_iter();
        let mut explicit_command = Vec::new();

        while let Some(arg) = iter.next() {
            let arg_str = arg.to_string_lossy();
            if arg_str == "--" {
                explicit_command.extend(iter);
                break;
            } else if arg_str == "--src-out" {
                if let Some(val) = iter.next() {
                    self.src_out = Some(PathBuf::from(val));
                } else {
                    return Err("Missing value for --src-out".to_string());
                }
            } else if let Some(val) = arg_str.strip_prefix("--src-out=") {
                self.src_out = Some(PathBuf::from(val));
            } else if arg_str == "--out" {
                if let Some(val) = iter.next() {
                    self.out = Some(PathBuf::from(val));
                } else {
                    return Err("Missing value for --out".to_string());
                }
            } else if let Some(val) = arg_str.strip_prefix("--out=") {
                self.out = Some(PathBuf::from(val));
            } else if arg_str == "-h" || arg_str == "--help" {
                return Err("HELP".to_string());
            } else {
                explicit_command.push(arg);
                explicit_command.extend(iter);
                break;
            }
        }

        if !explicit_command.is_empty() {
            if self.command.is_empty() {
                self.command = explicit_command;
            } else {
                self.command.extend(explicit_command);
            }
        }

        Ok(())
    }
}

pub fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

pub fn make_writable(path: &Path) -> io::Result<()> {
    if let Ok(metadata) = fs::metadata(path) {
        let mut perms = metadata.permissions();
        if perms.readonly() {
            perms.set_readonly(false);
            fs::set_permissions(path, perms)?;
        }
    }
    Ok(())
}

pub fn copy_file_safely(src: &Path, dst: &Path) -> io::Result<u64> {
    ensure_parent_dir(dst)?;
    if dst.exists() {
        let _ = make_writable(dst);
    }
    fs::copy(src, dst)
}

pub fn remove_file_safely(path: &Path) -> io::Result<()> {
    if path.exists() {
        let _ = make_writable(path);
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn execute_command(cmd: &OsStr, args: &[OsString]) -> io::Result<i32> {
    let mut child = Command::new(cmd);
    child.args(args);
    child.stdin(Stdio::inherit());
    child.stdout(Stdio::inherit());
    child.stderr(Stdio::inherit());

    let status = child.status()?;
    Ok(exit_code_from_status(status))
}

pub fn exit_code_from_status(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        code
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(sig) = status.signal() {
                128 + sig
            } else {
                1
            }
        }
        #[cfg(not(unix))]
        {
            1
        }
    }
}

pub fn run(config: &Config, in_workspace: bool) -> io::Result<i32> {
    if config.command.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "No command specified to execute",
        ));
    }

    let cmd = &config.command[0];
    let args = &config.command[1..];

    if in_workspace {
        return execute_command(cmd, args);
    }

    let src_out = config.src_out.as_deref();
    let out = config.out.as_deref();

    let src_out_exists = src_out.map(|p| p.exists()).unwrap_or(false);

    if src_out_exists {
        let src = src_out.unwrap();
        if let Some(dst) = out {
            copy_file_safely(src, dst)?;
        }
        remove_file_safely(src)?;
        if let Some(dst) = out {
            copy_file_safely(dst, src)?;
            let _ = make_writable(src);
        }

        let exit_code = execute_command(cmd, args)?;

        if let Some(dst) = out {
            if src.exists() {
                copy_file_safely(src, dst)?;
            }
        }

        Ok(exit_code)
    } else {
        let exit_code = execute_command(cmd, args)?;

        if let (Some(src), Some(dst)) = (src_out, out) {
            if src.exists() {
                copy_file_safely(src, dst)?;
            }
        }

        Ok(exit_code)
    }
}

pub fn print_help() {
    eprintln!(
        "Usage: uv_lock [OPTIONS] [--] <COMMAND> [ARGS...]\n\
         \n\
         Options:\n\
         \x20 --src-out <PATH>  Source tree lockfile path (or env UV_LOCK_SRC_OUT)\n\
         \x20 --out <PATH>      Target output file path (or env UV_LOCK_OUT)\n\
         \x20 -h, --help        Print this help message\n"
    );
}
