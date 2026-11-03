//! Library for building deterministic Python `.whl` archives (PEP 427).

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter, ZIP64_BYTES_THR};

pub const BLOCK_SIZE: usize = 256 * 1024;

/// Permissions `-rwxrwxrwx` with regular file bit (`0o777 | 0o100000`).
pub const REGULAR_FILE_UNIX_MODE: u32 = 0o100777;

const URLSAFE_B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

static RE_ESCAPE_SEGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w\d.]+").expect("valid escape segment regex"));

static RE_PACKAGE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[-_.]+").expect("valid package name regex"));

static RE_PEP440_VERSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?xi)
        ^\s*
        v?
        (?:
            (?:(?P<epoch>[0-9]+)!)?
            (?P<release>[0-9]+(?:\.[0-9]+)*)
            (?P<pre>
                [-_\.]?
                (?P<pre_l>alpha|a|beta|b|preview|pre|c|rc)
                [-_\.]?
                (?P<pre_n>[0-9]+)?
            )?
            (?P<post>
                (?:-(?P<post_n1>[0-9]+))
                |
                (?:
                    [-_\.]?
                    (?P<post_l>post|rev|r)
                    [-_\.]?
                    (?P<post_n2>[0-9]+)?
                )
            )?
            (?P<dev>
                [-_\.]?
                (?P<dev_l>dev)
                [-_\.]?
                (?P<dev_n>[0-9]+)?
            )?
        )
        (?:\+(?P<local>[a-z0-9]+(?:[-_\.][a-z0-9]+)*))?
        \s*$
        ",
    )
    .expect("valid PEP 440 version regex")
});

static RE_LOCAL_SANITIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("valid local sanitize regex"));

static RE_BRACE_PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\w+\}").expect("valid brace placeholder regex"));

static RE_METADATA_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^Name: .*$").expect("valid metadata Name regex"));

/// Command-line options for `wheelmaker`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WheelOptions {
    pub name: String,
    pub version: String,
    pub build_tag: String,
    pub python_tag: String,
    pub abi: String,
    pub platform: String,
    pub out: Option<PathBuf>,
    pub no_compress: bool,
    pub name_file: Option<PathBuf>,
    pub strip_path_prefix: Vec<String>,
    pub path_prefix: String,
    pub metadata_file: Option<PathBuf>,
    pub description_file: Option<PathBuf>,
    pub description_content_type: Option<String>,
    pub entry_points_file: Option<PathBuf>,
    pub input_file: Vec<String>,
    pub input_file_list: Vec<PathBuf>,
    pub extra_distinfo_file: Vec<String>,
    pub data_files: Vec<String>,
    pub volatile_status_file: Option<PathBuf>,
    pub stable_status_file: Option<PathBuf>,
    pub extra_requires_file: Vec<String>,
}

/// Returns the common prefix components between `path1` and `path2`.
pub fn commonpath(path1: &str, path2: &str) -> String {
    let sep = std::path::MAIN_SEPARATOR;
    let mut ret = Vec::new();
    for (a, b) in path1.split(sep).zip(path2.split(sep)) {
        if a != b {
            break;
        }
        ret.push(a);
    }
    let sep_str = sep.to_string();
    ret.join(&sep_str)
}

/// Escapes a filename segment per PEP 427 (`re.sub(r"[^\w\d.]+", "_", segment, re.UNICODE)`).
pub fn escape_filename_segment(segment: &str) -> String {
    RE_ESCAPE_SEGMENT.replace_all(segment, "_").into_owned()
}

/// Normalizes a package name (`re.sub(r"[-_.]+", "-", name).lower()`).
pub fn normalize_package_name(name: &str) -> String {
    RE_PACKAGE_NAME.replace_all(name, "-").to_lowercase()
}

/// Escapes the distribution name component of a filename (`normalize_package_name(name).replace("-", "_")`).
pub fn escape_filename_distribution_name(name: &str) -> String {
    normalize_package_name(name).replace('-', "_")
}

fn strip_leading_zeros(digits: &str) -> &str {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        "0"
    } else {
        trimmed
    }
}

/// Strictly parses and normalizes a PEP 440 version string, matching `str(packaging.version.Version(version))`.
pub fn try_normalize_pep440(version: &str) -> Result<String, String> {
    let caps = RE_PEP440_VERSION
        .captures(version)
        .ok_or_else(|| format!("Invalid version: '{}'", version))?;

    let mut out = String::new();

    if let Some(ep) = caps.name("epoch") {
        let ep_norm = strip_leading_zeros(ep.as_str());
        if ep_norm != "0" {
            out.push_str(ep_norm);
            out.push('!');
        }
    }

    let release = caps.name("release").expect("release group required");
    let rel_norm: Vec<&str> = release
        .as_str()
        .split('.')
        .map(strip_leading_zeros)
        .collect();
    out.push_str(&rel_norm.join("."));

    if let Some(pre_l) = caps.name("pre_l") {
        let norm_l = match pre_l.as_str().to_ascii_lowercase().as_str() {
            "alpha" | "a" => "a",
            "beta" | "b" => "b",
            "c" | "pre" | "preview" | "rc" => "rc",
            _ => unreachable!("constrained by RE_PEP440_VERSION"),
        };
        let pre_n = caps
            .name("pre_n")
            .map_or("0", |m| strip_leading_zeros(m.as_str()));
        out.push_str(norm_l);
        out.push_str(pre_n);
    }

    if caps.name("post").is_some() {
        let post_n = caps
            .name("post_n1")
            .or_else(|| caps.name("post_n2"))
            .map_or("0", |m| strip_leading_zeros(m.as_str()));
        out.push_str(".post");
        out.push_str(post_n);
    }

    if caps.name("dev_l").is_some() {
        let dev_n = caps
            .name("dev_n")
            .map_or("0", |m| strip_leading_zeros(m.as_str()));
        out.push_str(".dev");
        out.push_str(dev_n);
    }

    if let Some(local) = caps.name("local") {
        let parts: Vec<String> = local
            .as_str()
            .split(['.', '_', '-'])
            .map(|seg| {
                if seg.bytes().all(|b| b.is_ascii_digit()) {
                    strip_leading_zeros(seg).to_string()
                } else {
                    seg.to_ascii_lowercase()
                }
            })
            .collect();
        out.push('+');
        out.push_str(&parts.join("."));
    }

    Ok(out)
}

/// Normalizes a version string according to PEP 440, with fallback for `{PLACEHOLDER}` stamps.
pub fn normalize_pep440(version: &str) -> String {
    if let Ok(norm) = try_normalize_pep440(version) {
        return norm;
    }

    let lowered = version.to_lowercase();
    let sanitized = RE_LOCAL_SANITIZE
        .replace_all(&lowered, ".")
        .trim_matches('.')
        .to_string();
    let substituted = RE_BRACE_PLACEHOLDER.replace_all(version, "0");
    let delimiter = if substituted.contains('+') { "." } else { "+" };
    let candidate = if sanitized.is_empty() {
        substituted.into_owned()
    } else {
        format!("{}{}{}", substituted, delimiter, sanitized)
    };
    if let Ok(norm) = try_normalize_pep440(&candidate) {
        return norm;
    }

    let fallback = if sanitized.is_empty() {
        "0".to_string()
    } else {
        format!("0+{}", sanitized)
    };
    try_normalize_pep440(&fallback).unwrap_or_else(|_| "0".to_string())
}

/// Returns the within-archive name for a given file path name.
pub fn arcname_from(
    name: &str,
    distribution_prefix: &str,
    strip_path_prefixes: &[String],
    add_path_prefix: &str,
) -> String {
    let normalized_arcname = name.replace('\\', "/");
    if !distribution_prefix.is_empty() && normalized_arcname.starts_with(distribution_prefix) {
        return normalized_arcname;
    }
    for prefix in strip_path_prefixes {
        if let Some(stripped) = normalized_arcname.strip_prefix(prefix.as_str()) {
            return format!("{}{}", add_path_prefix, stripped);
        }
    }
    format!("{}{}", add_path_prefix, normalized_arcname)
}

/// Encodes `bytes` using URL-safe base64 without trailing `=` padding.
pub fn urlsafe_b64encode_nopad(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let b0 = chunk[0] as usize;
        let b1 = chunk[1] as usize;
        let b2 = chunk[2] as usize;
        out.push(URLSAFE_B64_ALPHABET[b0 >> 2] as char);
        out.push(URLSAFE_B64_ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        out.push(URLSAFE_B64_ALPHABET[((b1 & 0x0F) << 2) | (b2 >> 6)] as char);
        out.push(URLSAFE_B64_ALPHABET[b2 & 0x3F] as char);
    }
    let rem = chunks.remainder();
    if rem.len() == 1 {
        let b0 = rem[0] as usize;
        out.push(URLSAFE_B64_ALPHABET[b0 >> 2] as char);
        out.push(URLSAFE_B64_ALPHABET[(b0 & 0x03) << 4] as char);
    } else if rem.len() == 2 {
        let b0 = rem[0] as usize;
        let b1 = rem[1] as usize;
        out.push(URLSAFE_B64_ALPHABET[b0 >> 2] as char);
        out.push(URLSAFE_B64_ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        out.push(URLSAFE_B64_ALPHABET[(b1 & 0x0F) << 2] as char);
    }
    out
}

/// Formats a raw 32-byte SHA-256 digest as `sha256=<urlsafe_b64_nopad>`.
pub fn serialize_digest(sha256_bytes: &[u8]) -> String {
    format!("sha256={}", urlsafe_b64encode_nopad(sha256_bytes))
}

/// Quotes a filename for the `RECORD` file using CSV rules (`QUOTE_ALL` or `QUOTE_MINIMAL`).
pub fn quote_filename(filename: &str, quote_all_filenames: bool) -> String {
    let filename = filename.trim_start_matches('/');
    let needs_quotes = quote_all_filenames
        || filename.contains(',')
        || filename.contains('"')
        || filename.contains('\n')
        || filename.contains('\r');
    if needs_quotes {
        format!("\"{}\"", filename.replace('"', "\"\""))
    } else {
        filename.to_string()
    }
}

/// Collects input `(package_path, real_path)` pairs into a deduplicated map.
pub fn get_files_to_package(input_files: &[(String, String)]) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for (package_path, real_path) in input_files {
        files.insert(package_path.clone(), real_path.clone());
    }
    files
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MarkerToken {
    LParen,
    RParen,
    Ident(String),
    Str(String),
    Op(String),
    And,
    Or,
}

fn tokenize_marker(marker: &str) -> Result<Vec<MarkerToken>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = marker.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '(' {
            tokens.push(MarkerToken::LParen);
            i += 1;
            continue;
        }
        if c == ')' {
            tokens.push(MarkerToken::RParen);
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            let quote = c;
            i += 1;
            let mut val = String::new();
            let mut closed = false;
            while i < chars.len() {
                if chars[i] == quote {
                    closed = true;
                    i += 1;
                    break;
                }
                val.push(chars[i]);
                i += 1;
            }
            if !closed {
                return Err(format!("Unclosed quote in marker: {}", marker));
            }
            tokens.push(MarkerToken::Str(val));
            continue;
        }
        let rem: String = chars[i..].iter().collect();
        let mut matched_op = None;
        for op in ["===", "~=", "==", "!=", "<=", ">=", "<", ">"] {
            if rem.starts_with(op) {
                matched_op = Some(op);
                break;
            }
        }
        if let Some(op) = matched_op {
            tokens.push(MarkerToken::Op(op.to_string()));
            i += op.len();
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '_'
                    || chars[i] == '.'
                    || chars[i] == '-')
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            match word.as_str() {
                "and" => tokens.push(MarkerToken::And),
                "or" => tokens.push(MarkerToken::Or),
                "in" => tokens.push(MarkerToken::Op("in".to_string())),
                "not" => {
                    while i < chars.len() && chars[i].is_whitespace() {
                        i += 1;
                    }
                    let rem_after: String = chars[i..].iter().collect();
                    if rem_after.starts_with("in")
                        && !rem_after[2..]
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                    {
                        i += 2;
                        tokens.push(MarkerToken::Op("not in".to_string()));
                    } else {
                        return Err(format!("Unexpected 'not' in marker: {}", marker));
                    }
                }
                _ => tokens.push(MarkerToken::Ident(word)),
            }
            continue;
        }
        return Err(format!(
            "Unexpected character '{}' in marker: {}",
            c, marker
        ));
    }
    Ok(tokens)
}

fn normalize_marker(marker: &str) -> Result<String, String> {
    let tokens = tokenize_marker(marker)?;
    if tokens.is_empty() {
        return Err("Empty marker".to_string());
    }
    let mut out = String::new();
    for (idx, tok) in tokens.iter().enumerate() {
        match tok {
            MarkerToken::LParen => out.push('('),
            MarkerToken::RParen => out.push(')'),
            MarkerToken::Ident(id) => out.push_str(id),
            MarkerToken::Str(val) => {
                let is_extra_rhs = idx >= 2
                    && matches!(tokens.get(idx - 1), Some(MarkerToken::Op(_)))
                    && matches!(tokens.get(idx - 2), Some(MarkerToken::Ident(id)) if id == "extra");
                let is_extra_lhs = matches!(tokens.get(idx + 1), Some(MarkerToken::Op(_)))
                    && matches!(tokens.get(idx + 2), Some(MarkerToken::Ident(id)) if id == "extra");
                let formatted = if is_extra_rhs || is_extra_lhs {
                    normalize_package_name(val)
                } else {
                    val.clone()
                };
                out.push('"');
                out.push_str(&formatted);
                out.push('"');
            }
            MarkerToken::Op(op) => {
                out.push(' ');
                out.push_str(op);
                out.push(' ');
            }
            MarkerToken::And => out.push_str(" and "),
            MarkerToken::Or => out.push_str(" or "),
        }
    }
    Ok(out)
}

fn find_unquoted_semicolon(s: &str) -> Option<usize> {
    let mut in_quote: Option<char> = None;
    for (idx, c) in s.char_indices() {
        if let Some(q) = in_quote {
            if c == q {
                in_quote = None;
            }
        } else if c == '\'' || c == '"' {
            in_quote = Some(c);
        } else if c == ';' {
            return Some(idx);
        }
    }
    None
}

fn normalize_specifier_set(spec_str: &str) -> Result<String, String> {
    let mut s = spec_str.trim();
    if s.starts_with('(') && s.ends_with(')') {
        s = s[1..s.len() - 1].trim();
    }
    if s.is_empty() {
        return Ok(String::new());
    }
    let mut specs = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut matched = None;
        for op in ["===", "~=", "==", "!=", "<=", ">=", "<", ">"] {
            if let Some(rest) = part.strip_prefix(op) {
                matched = Some((op, rest.trim()));
                break;
            }
        }
        let (op, ver) = matched.ok_or_else(|| format!("Invalid version specifier: '{}'", part))?;
        if ver.is_empty() {
            return Err(format!("Missing version in specifier: '{}'", part));
        }
        specs.push(format!("{}{}", op, ver));
    }
    specs.sort();
    specs.dedup();
    Ok(specs.join(","))
}

/// Formats a PEP 508 requirement text into a normalized `Requires-Dist` metadata line.
pub fn get_new_requirement_line(reqs_text: &str, extra: &str) -> Result<String, String> {
    let s = reqs_text.trim();
    if s.is_empty() {
        return Err("Empty requirement".to_string());
    }

    let (before_marker, marker_part) = if let Some(semi_idx) = find_unquoted_semicolon(s) {
        (&s[..semi_idx], Some(s[semi_idx + 1..].trim()))
    } else {
        (s, None)
    };

    let before_marker = before_marker.trim();
    let name_len = before_marker
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .map(|c| c.len_utf8())
        .sum::<usize>();
    if name_len == 0 {
        return Err(format!("Invalid requirement name in '{}'", reqs_text));
    }
    let name = &before_marker[..name_len];
    let mut rest = before_marker[name_len..].trim_start();

    let req_extra_deps = if rest.starts_with('[') {
        let close_idx = rest
            .find(']')
            .ok_or_else(|| format!("Unclosed extras bracket in '{}'", reqs_text))?;
        let extras_inner = &rest[1..close_idx];
        let mut extras: Vec<&str> = extras_inner
            .split(',')
            .map(|e| e.trim())
            .filter(|e| !e.is_empty())
            .collect();
        extras.sort_unstable();
        extras.dedup();
        rest = rest[close_idx + 1..].trim_start();
        if extras.is_empty() {
            String::new()
        } else {
            format!("[{}]", extras.join(","))
        }
    } else {
        String::new()
    };

    let req_spec = if let Some(after_at) = rest.strip_prefix('@') {
        let url = after_at.trim();
        if url.is_empty() {
            return Err(format!("Empty URL in requirement '{}'", reqs_text));
        }
        format!(" @ {}", url)
    } else {
        normalize_specifier_set(rest)?
    };

    let base = format!("Requires-Dist: {}{}{}", name, req_extra_deps, req_spec);
    let normalized_marker = match marker_part {
        Some(m) if !m.is_empty() => Some(normalize_marker(m)?),
        _ => None,
    };

    if let Some(marker) = normalized_marker {
        if !extra.is_empty() {
            Ok(format!("{}; ({}) and {}", base, marker, extra))
        } else {
            Ok(format!("{}; {}", base, marker))
        }
    } else if !extra.is_empty() {
        Ok(format!("{}; {}", base, extra))
    } else {
        Ok(base)
    }
}

/// Resolves `{KEY}` workspace status stamps in `argument` using pre-read status lines.
pub fn resolve_stamp_with_lines(argument: &str, status_lines: &[String]) -> String {
    let mut resolved = argument.to_string();
    for line in status_lines {
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.split_once(' ') {
            Some((k, v)) => (k, v),
            None => (line.as_str(), ""),
        };
        let stamp = format!("{{{}}}", key);
        resolved = resolved.replace(&stamp, value);
    }
    resolved
}

/// Resolves workspace status stamps found in `argument` from volatile and stable status files.
pub fn resolve_argument_stamp(
    argument: &str,
    volatile_status_stamp: &Path,
    stable_status_stamp: &Path,
) -> io::Result<String> {
    let volatile_text = fs::read_to_string(volatile_status_stamp)?;
    let stable_text = fs::read_to_string(stable_status_stamp)?;
    let lines: Vec<String> = volatile_text
        .lines()
        .chain(stable_text.lines())
        .map(|l| l.to_string())
        .collect();
    Ok(resolve_stamp_with_lines(argument, &lines))
}

/// Low-level `.whl` archive writer tracking `RECORD` entries, mirroring `_WhlFile` in `wheelmaker.py`.
pub struct WhlFile<W: Write + Seek> {
    zip: ZipWriter<W>,
    compress: bool,
    distribution_prefix: String,
    strip_path_prefixes: Vec<String>,
    add_path_prefix: String,
    record: Vec<(String, String, String)>,
    pub quote_all_filenames: bool,
}

impl<W: Write + Seek> WhlFile<W> {
    pub fn new(
        writer: W,
        distribution_prefix: &str,
        strip_path_prefixes: Vec<String>,
        add_path_prefix: &str,
        compress: bool,
        quote_all_filenames: bool,
    ) -> Self {
        Self {
            zip: ZipWriter::new(writer),
            compress,
            distribution_prefix: distribution_prefix.to_string(),
            strip_path_prefixes,
            add_path_prefix: add_path_prefix.to_string(),
            record: Vec::new(),
            quote_all_filenames,
        }
    }

    pub fn distinfo_path(&self, basename: &str) -> String {
        format!("{}.dist-info/{}", self.distribution_prefix, basename)
    }

    pub fn data_path(&self, basename: &str) -> String {
        format!("{}.data/{}", self.distribution_prefix, basename)
    }

    pub fn quote_filename(&self, filename: &str) -> String {
        quote_filename(filename, self.quote_all_filenames)
    }

    fn clean_zip_arcname(filename: &str) -> &str {
        filename.trim_start_matches(['/', '\\'])
    }

    fn file_options(&self, size: u64) -> SimpleFileOptions {
        let method = if self.compress {
            CompressionMethod::Deflated
        } else {
            CompressionMethod::Stored
        };
        SimpleFileOptions::default()
            .compression_method(method)
            .last_modified_time(DateTime::default())
            .unix_permissions(REGULAR_FILE_UNIX_MODE)
            .large_file(size >= ZIP64_BYTES_THR)
    }

    pub fn add_file(&mut self, package_filename: &str, real_filename: &Path) -> io::Result<()> {
        if real_filename.is_dir() {
            let mut entries = Vec::new();
            for entry in fs::read_dir(real_filename)? {
                let entry = entry?;
                let file_name = entry.file_name().to_string_lossy().into_owned();
                entries.push((file_name, entry.path()));
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (file_name, path) in entries {
                let child_pkg = format!("{}/{}", package_filename, file_name);
                self.add_file(&child_pkg, &path)?;
            }
            return Ok(());
        }

        let arcname = arcname_from(
            package_filename,
            &self.distribution_prefix,
            &self.strip_path_prefixes,
            &self.add_path_prefix,
        );
        let zip_arcname = Self::clean_zip_arcname(&arcname);

        let mut src = File::open(real_filename)?;
        let initial_len = src.metadata()?.len();
        let opts = self.file_options(initial_len);
        self.zip
            .start_file(zip_arcname, opts)
            .map_err(io::Error::from)?;

        let mut sha = Sha256::new();
        let mut size: u64 = 0;
        let mut buf = vec![0u8; BLOCK_SIZE];
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            self.zip.write_all(&buf[..n])?;
            sha.update(&buf[..n]);
            size += n as u64;
        }

        let digest = serialize_digest(&sha.finalize());
        self.record.push((arcname, digest, size.to_string()));
        Ok(())
    }

    pub fn add_string(&mut self, filename: &str, contents: &[u8]) -> io::Result<()> {
        let zip_arcname = Self::clean_zip_arcname(filename);
        let opts = self.file_options(contents.len() as u64);
        self.zip
            .start_file(zip_arcname, opts)
            .map_err(io::Error::from)?;
        self.zip.write_all(contents)?;

        let mut sha = Sha256::new();
        sha.update(contents);
        let digest = serialize_digest(&sha.finalize());
        self.record
            .push((filename.to_string(), digest, contents.len().to_string()));
        Ok(())
    }

    pub fn add_recordfile(&mut self) -> io::Result<String> {
        let record_path = self.distinfo_path("RECORD");
        let mut lines = Vec::with_capacity(self.record.len() + 1);
        for (fname, digest, size) in &self.record {
            lines.push(format!(
                "{},{},{}",
                self.quote_filename(fname),
                digest,
                size
            ));
        }
        lines.push(format!("{},,", self.quote_filename(&record_path)));
        let contents = format!("{}\n", lines.join("\n"));
        self.add_string(&record_path, contents.as_bytes())?;
        Ok(contents)
    }

    pub fn finish(self) -> io::Result<W> {
        let mut inner = self.zip.finish().map_err(io::Error::from)?;
        inner.flush()?;
        Ok(inner)
    }
}

/// High-level wheel builder mirroring `WheelMaker` in `tools/wheelmaker.py`.
pub struct WheelMaker {
    name: String,
    version: String,
    build_tag: String,
    python_tag: String,
    abi: String,
    platform: String,
    outfile: Option<PathBuf>,
    strip_path_prefixes: Vec<String>,
    add_path_prefix: String,
    compress: bool,
    wheelname_fragment_distribution_name: String,
    distribution_prefix: String,
}

impl WheelMaker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        version: &str,
        build_tag: &str,
        python_tag: &str,
        abi: &str,
        platform: &str,
        compress: bool,
        outfile: Option<PathBuf>,
        strip_path_prefixes: Vec<String>,
        add_path_prefix: &str,
    ) -> Self {
        let norm_version = normalize_pep440(version);
        let wheelname_fragment_distribution_name = escape_filename_distribution_name(name);
        let distribution_prefix =
            format!("{}-{}", wheelname_fragment_distribution_name, norm_version);
        Self {
            name: name.to_string(),
            version: norm_version,
            build_tag: build_tag.to_string(),
            python_tag: python_tag.to_string(),
            abi: abi.to_string(),
            platform: platform.to_string(),
            outfile,
            strip_path_prefixes,
            add_path_prefix: add_path_prefix.to_string(),
            compress,
            wheelname_fragment_distribution_name,
            distribution_prefix,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn wheelname(&self) -> String {
        let mut components = vec![
            self.wheelname_fragment_distribution_name.as_str(),
            self.version.as_str(),
        ];
        if !self.build_tag.is_empty() {
            components.push(self.build_tag.as_str());
        }
        components.extend([
            self.python_tag.as_str(),
            self.abi.as_str(),
            self.platform.as_str(),
        ]);
        format!("{}.whl", components.join("-"))
    }

    pub fn filename(&self) -> PathBuf {
        if let Some(ref out) = self.outfile {
            out.clone()
        } else {
            PathBuf::from(self.wheelname())
        }
    }

    pub fn disttags(&self) -> Vec<String> {
        vec![format!(
            "{}-{}-{}",
            self.python_tag, self.abi, self.platform
        )]
    }

    pub fn distinfo_path(&self, basename: &str) -> String {
        format!("{}.dist-info/{}", self.distribution_prefix, basename)
    }

    pub fn data_path(&self, basename: &str) -> String {
        format!("{}.data/{}", self.distribution_prefix, basename)
    }

    pub fn wheelfile_contents(&self) -> String {
        let purelib = if self.platform == "any" {
            "true"
        } else {
            "false"
        };
        let mut contents = format!(
            "Wheel-Version: 1.0\nGenerator: bazel-wheelmaker 1.0\nRoot-Is-Purelib: {}\n",
            purelib
        );
        for tag in self.disttags() {
            contents.push_str(&format!("Tag: {}\n", tag));
        }
        contents
    }

    pub fn format_metadata(&self, metadata: &str, name: &str, description: Option<&str>) -> String {
        let replacement = format!("Name: {}", name);
        let mut out = RE_METADATA_NAME
            .replace_all(metadata, regex::NoExpand(&replacement))
            .into_owned();
        out.push_str(&format!("Version: {}\n\n", self.version));
        match description {
            Some(desc) if !desc.is_empty() => out.push_str(desc),
            _ => out.push_str("UNKNOWN"),
        }
        out.push('\n');
        out
    }

    pub fn open_whlfile<W: Write + Seek>(&self, writer: W) -> WhlFile<W> {
        WhlFile::new(
            writer,
            &self.distribution_prefix,
            self.strip_path_prefixes.clone(),
            &self.add_path_prefix,
            self.compress,
            false,
        )
    }
}

fn parse_file_pairs(items: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::with_capacity(items.len());
    for item in items {
        let (left, right) = item
            .split_once(';')
            .ok_or_else(|| format!("expected 'target;real_path' pair, got '{}'", item))?;
        pairs.push((left.to_string(), right.to_string()));
    }
    Ok(pairs)
}

fn read_text_universal_newlines(path: &Path) -> io::Result<String> {
    let raw = fs::read_to_string(path)?;
    Ok(raw.replace("\r\n", "\n"))
}

/// Expands `Requires-Dist:` lines in `metadata` (both inline with markers and `@file` references).
pub fn expand_metadata_requires(
    metadata: &str,
    extra_requires_map: &HashMap<String, String>,
) -> io::Result<String> {
    let mut result = metadata.to_string();
    let lines: Vec<String> = metadata.lines().map(|l| l.to_string()).collect();

    for meta_line in lines {
        let Some(after_prefix) = meta_line.strip_prefix("Requires-Dist: ") else {
            continue;
        };

        if let Some(after_at) = after_prefix.strip_prefix('@') {
            let (file_part, extra_part) = match after_at.split_once(';') {
                Some((f, e)) => (f, e.trim()),
                None => (after_at, ""),
            };
            let extra = if !extra_part.is_empty() {
                extra_part.to_string()
            } else if let Some(mapped) = extra_requires_map.get(file_part.trim()) {
                mapped.clone()
            } else {
                String::new()
            };

            let req_file_text = read_text_universal_newlines(Path::new(file_part))?;
            let mut reqs = Vec::new();
            for reqs_line in req_file_text.lines() {
                let reqs_text = reqs_line.trim();
                if reqs_text.is_empty() || reqs_text.starts_with('#') || reqs_text.starts_with('-')
                {
                    continue;
                }
                let reqs_text = match reqs_text.split_once('#') {
                    Some((before_comment, _)) => before_comment,
                    None => reqs_text,
                };
                let formatted = get_new_requirement_line(reqs_text, &extra)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                reqs.push(formatted);
            }

            if !reqs.is_empty() {
                result = result.replace(&meta_line, &reqs.join("\n"));
            } else {
                let empty_line_re =
                    Regex::new(&format!(r"{}(?:\r?\n)?", regex::escape(&meta_line)))
                        .expect("escaped literal regex is always valid");
                result = empty_line_re.replace(&result, "").into_owned();
            }
        } else {
            let Some((package, extra)) = after_prefix.rsplit_once(';') else {
                continue;
            };
            if package.is_empty() {
                continue;
            }
            let extra = extra.trim();
            let formatted = get_new_requirement_line(package, extra)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            result = result.replace(&meta_line, &formatted);
        }
    }

    Ok(result)
}

/// Builds the wheel archive and optional `--name_file` according to `options`.
pub fn build_wheel(options: &WheelOptions) -> io::Result<()> {
    let mut input_files = parse_file_pairs(&options.input_file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut extra_distinfo_file = parse_file_pairs(&options.extra_distinfo_file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut data_files = parse_file_pairs(&options.data_files)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    for list_path in &options.input_file_list {
        let content = read_text_universal_newlines(list_path)?;
        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            let (pkg, real) = line.split_once(';').ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "invalid line in input_file_list {}: '{}'",
                        list_path.display(),
                        line
                    ),
                )
            })?;
            input_files.push((pkg.to_string(), real.to_string()));
        }
    }

    let all_files = get_files_to_package(&input_files);

    let mut status_lines = Vec::new();
    if let (Some(ref volatile_path), Some(ref stable_path)) =
        (&options.volatile_status_file, &options.stable_status_file)
    {
        let volatile_text = read_text_universal_newlines(volatile_path)?;
        let stable_text = read_text_universal_newlines(stable_path)?;
        status_lines.extend(volatile_text.lines().map(|s| s.to_string()));
        status_lines.extend(stable_text.lines().map(|s| s.to_string()));
    } else {
        if let Some(ref volatile_path) = options.volatile_status_file {
            let volatile_text = read_text_universal_newlines(volatile_path)?;
            status_lines.extend(volatile_text.lines().map(|s| s.to_string()));
        }
        if let Some(ref stable_path) = options.stable_status_file {
            let stable_text = read_text_universal_newlines(stable_path)?;
            status_lines.extend(stable_text.lines().map(|s| s.to_string()));
        }
    }

    let (name, version, build_tag) = if !status_lines.is_empty() {
        (
            resolve_stamp_with_lines(&options.name, &status_lines),
            resolve_stamp_with_lines(&options.version, &status_lines),
            resolve_stamp_with_lines(&options.build_tag, &status_lines),
        )
    } else {
        (
            options.name.clone(),
            options.version.clone(),
            options.build_tag.clone(),
        )
    };

    let maker = WheelMaker::new(
        &name,
        &version,
        &build_tag,
        &options.python_tag,
        &options.abi,
        &options.platform,
        !options.no_compress,
        options.out.clone(),
        options.strip_path_prefix.clone(),
        &options.path_prefix,
    );

    let out_path = maker.filename();
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let out_file = BufWriter::with_capacity(BLOCK_SIZE, File::create(&out_path)?);
    let mut whl = maker.open_whlfile(out_file);

    for (package_filename, real_filename) in &all_files {
        whl.add_file(package_filename, Path::new(real_filename))?;
    }

    let wheel_contents = maker.wheelfile_contents();
    whl.add_string(&maker.distinfo_path("WHEEL"), wheel_contents.as_bytes())?;

    let description = if let Some(ref desc_path) = options.description_file {
        Some(read_text_universal_newlines(desc_path)?)
    } else {
        None
    };

    let metadata_path = options.metadata_file.as_ref().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "--metadata_file is required")
    })?;
    let mut raw_metadata = read_text_universal_newlines(metadata_path)?;
    if !status_lines.is_empty() {
        raw_metadata = resolve_stamp_with_lines(&raw_metadata, &status_lines);
    }

    let mut extra_requires_map = HashMap::new();
    for item in &options.extra_requires_file {
        if let Some((file, extra_opt)) = item.split_once(';') {
            let formatted_extra = if extra_opt.trim().starts_with("extra") {
                extra_opt.trim().to_string()
            } else {
                format!("extra == '{}'", extra_opt.trim())
            };
            extra_requires_map.insert(file.trim().to_string(), formatted_extra);
        }
    }

    let expanded_metadata = expand_metadata_requires(&raw_metadata, &extra_requires_map)?;
    let final_metadata = maker.format_metadata(&expanded_metadata, &name, description.as_deref());
    whl.add_string(&maker.distinfo_path("METADATA"), final_metadata.as_bytes())?;

    if let Some(ref ep_path) = options.entry_points_file {
        whl.add_file(&maker.distinfo_path("entry_points.txt"), ep_path)?;
    }

    data_files.sort();
    for (filename, real_path) in &data_files {
        whl.add_file(&maker.data_path(filename), Path::new(real_path))?;
    }

    extra_distinfo_file.sort();
    for (filename, real_path) in &extra_distinfo_file {
        whl.add_file(&maker.distinfo_path(filename), Path::new(real_path))?;
    }

    whl.add_recordfile()?;
    whl.finish()?;

    if let Some(ref name_file_path) = options.name_file {
        if let Some(parent) = name_file_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(name_file_path, maker.wheelname())?;
    }

    Ok(())
}

fn expand_flagfiles<I, S>(args: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut expanded = Vec::new();
    for arg in args {
        let s = arg.as_ref();
        if let Some(file_path) = s.strip_prefix('@') {
            let content = fs::read_to_string(file_path)
                .map_err(|e| format!("failed to read flagfile '{}': {}", file_path, e))?;
            for line in content.lines() {
                expanded.push(line.to_string());
            }
        } else {
            expanded.push(s.to_string());
        }
    }
    Ok(expanded)
}

fn next_val(
    flag: &str,
    inline: Option<&str>,
    iter: &mut std::vec::IntoIter<String>,
) -> Result<String, String> {
    if let Some(v) = inline {
        Ok(v.to_string())
    } else {
        iter.next()
            .ok_or_else(|| format!("missing value for {}", flag))
    }
}

/// Parses command-line arguments (excluding `argv[0]`) into `WheelOptions`.
pub fn parse_args<I, S>(args: I) -> Result<WheelOptions, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let expanded = expand_flagfiles(args)?;
    let mut opts = WheelOptions {
        python_tag: "py3".to_string(),
        abi: "none".to_string(),
        platform: "any".to_string(),
        ..Default::default()
    };

    let mut has_name = false;
    let mut has_version = false;

    let mut iter = expanded.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "-h" || arg == "--help" {
            return Err("Usage: wheelmaker --name NAME --version VERSION [OPTIONS]".to_string());
        }

        let (flag, inline_val) = if let Some((f, v)) = arg.split_once('=') {
            if f.starts_with('-') {
                (f, Some(v))
            } else {
                (arg.as_str(), None)
            }
        } else {
            (arg.as_str(), None)
        };

        match flag {
            "--name" => {
                opts.name = next_val(flag, inline_val, &mut iter)?;
                has_name = true;
            }
            "--version" => {
                opts.version = next_val(flag, inline_val, &mut iter)?;
                has_version = true;
            }
            "--build_tag" => {
                opts.build_tag = next_val(flag, inline_val, &mut iter)?;
            }
            "--python_tag" => {
                opts.python_tag = next_val(flag, inline_val, &mut iter)?;
            }
            "--abi" => {
                opts.abi = next_val(flag, inline_val, &mut iter)?;
            }
            "--platform" => {
                opts.platform = next_val(flag, inline_val, &mut iter)?;
            }
            "--out" => {
                opts.out = Some(PathBuf::from(next_val(flag, inline_val, &mut iter)?));
            }
            "--no_compress" => {
                opts.no_compress = true;
            }
            "--Compress" | "--compress" => {
                opts.no_compress = false;
            }
            "--name_file" => {
                opts.name_file = Some(PathBuf::from(next_val(flag, inline_val, &mut iter)?));
            }
            "--strip_path_prefix" => {
                opts.strip_path_prefix
                    .push(next_val(flag, inline_val, &mut iter)?);
            }
            "--path_prefix" => {
                opts.path_prefix = next_val(flag, inline_val, &mut iter)?;
            }
            "--metadata_file" => {
                opts.metadata_file = Some(PathBuf::from(next_val(flag, inline_val, &mut iter)?));
            }
            "--description_file" => {
                opts.description_file = Some(PathBuf::from(next_val(flag, inline_val, &mut iter)?));
            }
            "--description_content_type" => {
                opts.description_content_type = Some(next_val(flag, inline_val, &mut iter)?);
            }
            "--entry_points_file" => {
                opts.entry_points_file =
                    Some(PathBuf::from(next_val(flag, inline_val, &mut iter)?));
            }
            "--input_file" => {
                opts.input_file.push(next_val(flag, inline_val, &mut iter)?);
            }
            "--input_file_list" => {
                opts.input_file_list
                    .push(PathBuf::from(next_val(flag, inline_val, &mut iter)?));
            }
            "--extra_distinfo_file" => {
                opts.extra_distinfo_file
                    .push(next_val(flag, inline_val, &mut iter)?);
            }
            "--data_files" => {
                opts.data_files.push(next_val(flag, inline_val, &mut iter)?);
            }
            "--volatile_status_file" => {
                opts.volatile_status_file =
                    Some(PathBuf::from(next_val(flag, inline_val, &mut iter)?));
            }
            "--stable_status_file" => {
                opts.stable_status_file =
                    Some(PathBuf::from(next_val(flag, inline_val, &mut iter)?));
            }
            "--extra_requires_file" => {
                opts.extra_requires_file
                    .push(next_val(flag, inline_val, &mut iter)?);
            }
            other => {
                return Err(format!("unrecognized argument: {}", other));
            }
        }
    }

    if !has_name {
        return Err("missing required argument: --name".to_string());
    }
    if !has_version {
        return Err("missing required argument: --version".to_string());
    }

    Ok(opts)
}

/// Parses CLI arguments and builds the wheel.
pub fn run<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let options = parse_args(args)?;
    build_wheel(&options).map_err(|e| e.to_string())
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), builds the wheel, and returns
/// the process exit code.
pub fn run_cli<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let str_args: Vec<String> = args
        .into_iter()
        .map(|a| a.into().to_string_lossy().into_owned())
        .collect();
    if let Err(err) = run(&str_args) {
        eprintln!("wheelmaker: error: {}", err);
        return 1;
    }
    0
}
