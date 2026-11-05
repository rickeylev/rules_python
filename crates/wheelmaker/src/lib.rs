//! Library for building deterministic Python `.whl` archives (PEP 427).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::LazyLock;

use base64::Engine as _;
use regex::Regex;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZIP64_BYTES_THR, ZipWriter};

pub const BLOCK_SIZE: usize = 256 * 1024;

/// Permissions `-rwxrwxrwx` with regular file bit (`0o777 | 0o100000`).
pub const REGULAR_FILE_UNIX_MODE: u32 = 0o100777;

static RE_ESCAPE_SEGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w\d.]+").expect("valid escape segment regex"));

static RE_PACKAGE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[-_.]+").expect("valid package name regex"));

static RE_LOCAL_SANITIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("valid local sanitize regex"));

static RE_BRACE_PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\w+\}").expect("valid brace placeholder regex"));

static RE_METADATA_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^Name: .*$").expect("valid metadata Name regex"));

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

/// Strictly parses and normalizes a PEP 440 version string.
pub fn try_normalize_pep440(version: &str) -> Result<String, String> {
    pep440_rs::Version::from_str(version.trim())
        .map(|ver| ver.to_string())
        .map_err(|err| err.to_string())
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
        format!("{substituted}{delimiter}{sanitized}")
    };
    if let Ok(norm) = try_normalize_pep440(&candidate) {
        return norm;
    }

    let fallback = if sanitized.is_empty() {
        "0".to_string()
    } else {
        format!("0+{sanitized}")
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
            return format!("{add_path_prefix}{stripped}");
        }
    }
    format!("{add_path_prefix}{normalized_arcname}")
}

/// Formats a raw 32-byte SHA-256 digest as `sha256=<urlsafe_b64_nopad>`.
pub fn serialize_digest(sha256_bytes: &[u8]) -> String {
    format!(
        "sha256={}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha256_bytes)
    )
}

/// Collects input `(package_path, real_path)` pairs into a deduplicated map.
pub fn get_files_to_package(input_files: &[(String, PathBuf)]) -> HashMap<String, PathBuf> {
    let mut files = HashMap::with_capacity(input_files.len());
    for (package_path, real_path) in input_files {
        files.insert(package_path.clone(), real_path.clone());
    }
    files
}

fn format_requirement(req: &pep508_rs::Requirement<pep508_rs::VerbatimUrl>) -> String {
    let mut out = format!("Requires-Dist: {}", req.name);
    if !req.extras.is_empty() {
        let mut extras: Vec<String> = req.extras.iter().map(ToString::to_string).collect();
        extras.sort_unstable();
        extras.dedup();
        out.push('[');
        out.push_str(&extras.join(","));
        out.push(']');
    }
    let mut is_url = false;
    if let Some(ref version_or_url) = req.version_or_url {
        match version_or_url {
            pep508_rs::VersionOrUrl::VersionSpecifier(specs) => {
                let mut spec_strs: Vec<String> = specs.iter().map(ToString::to_string).collect();
                spec_strs.sort_unstable();
                spec_strs.dedup();
                out.push_str(&spec_strs.join(","));
            }
            pep508_rs::VersionOrUrl::Url(url) => {
                is_url = true;
                out.push_str(" @ ");
                if let Some(given) = url.given() {
                    out.push_str(given);
                } else {
                    out.push_str(&url.to_string());
                }
            }
        }
    }
    if let Some(marker) = req.marker.contents() {
        if is_url {
            out.push_str(&format!(" ; {marker}"));
        } else {
            out.push_str(&format!("; {marker}"));
        }
    }
    out
}

/// Formats a PEP 508 requirement text into a normalized `Requires-Dist` metadata line.
pub fn get_new_requirement_line(reqs_text: &str, extra: &str) -> Result<String, String> {
    let mut req = pep508_rs::Requirement::<pep508_rs::VerbatimUrl>::from_str(reqs_text.trim())
        .map_err(|err| err.to_string())?;
    let extra_trimmed = extra.trim();
    if !extra_trimmed.is_empty() {
        let extra_tree =
            pep508_rs::MarkerTree::from_str(extra_trimmed).map_err(|err| err.to_string())?;
        req.marker.and(extra_tree);
    }
    Ok(format_requirement(&req))
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
        let stamp = format!("{{{key}}}");
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
        .map(ToString::to_string)
        .collect();
    Ok(resolve_stamp_with_lines(argument, &lines))
}

fn csv_quote_always(field: &str) -> io::Result<String> {
    let mut wtr = csv::WriterBuilder::new()
        .has_headers(false)
        .terminator(csv::Terminator::Any(b'\n'))
        .quote_style(csv::QuoteStyle::Always)
        .from_writer(Vec::new());
    wtr.write_record([field]).map_err(io::Error::other)?;
    let mut bytes = wtr.into_inner().map_err(io::Error::other)?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    String::from_utf8(bytes).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
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
        format!("{}.dist-info/{basename}", self.distribution_prefix)
    }

    pub fn data_path(&self, basename: &str) -> String {
        format!("{}.data/{basename}", self.distribution_prefix)
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
                let file_name = entry.file_name().into_string().map_err(|name| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("non-UTF-8 filename in input directory: {name:?}"),
                    )
                })?;
                entries.push((file_name, entry.path()));
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (file_name, path) in entries {
                let child_pkg = format!("{package_filename}/{file_name}");
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
        let contents = if self.quote_all_filenames {
            let mut out = String::new();
            for (fname, digest, size) in &self.record {
                let quoted = csv_quote_always(fname.trim_start_matches('/'))?;
                out.push_str(&format!("{quoted},{digest},{size}\n"));
            }
            let quoted_record = csv_quote_always(record_path.trim_start_matches('/'))?;
            out.push_str(&format!("{quoted_record},,\n"));
            out
        } else {
            let mut wtr = csv::WriterBuilder::new()
                .has_headers(false)
                .terminator(csv::Terminator::Any(b'\n'))
                .quote_style(csv::QuoteStyle::Necessary)
                .from_writer(Vec::new());
            for (fname, digest, size) in &self.record {
                wtr.write_record([
                    fname.trim_start_matches('/'),
                    digest.as_str(),
                    size.as_str(),
                ])
                .map_err(io::Error::other)?;
            }
            wtr.write_record([record_path.trim_start_matches('/'), "", ""])
                .map_err(io::Error::other)?;
            let bytes = wtr.into_inner().map_err(io::Error::other)?;
            String::from_utf8(bytes)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?
        };
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
        let distribution_prefix = format!("{wheelname_fragment_distribution_name}-{norm_version}");
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
        format!("{}.dist-info/{basename}", self.distribution_prefix)
    }

    pub fn data_path(&self, basename: &str) -> String {
        format!("{}.data/{basename}", self.distribution_prefix)
    }

    pub fn wheelfile_contents(&self) -> String {
        let purelib = if self.platform == "any" {
            "true"
        } else {
            "false"
        };
        let mut contents = format!(
            "Wheel-Version: 1.0\nGenerator: bazel-wheelmaker 1.0\nRoot-Is-Purelib: {purelib}\n"
        );
        for tag in self.disttags() {
            contents.push_str(&format!("Tag: {tag}\n"));
        }
        contents
    }

    pub fn format_metadata(&self, metadata: &str, name: &str, description: Option<&str>) -> String {
        let replacement = format!("Name: {name}");
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
    let lines: Vec<String> = metadata.lines().map(ToString::to_string).collect();

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
    let mut input_files = options.input_file.clone();
    let mut extra_distinfo_file = options.extra_distinfo_file.clone();
    let mut data_files = options.data_files.clone();

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
            input_files.push((pkg.to_string(), PathBuf::from(real)));
        }
    }

    let all_files = get_files_to_package(&input_files);
    let mut sorted_files: Vec<(String, PathBuf)> = all_files.into_iter().collect();
    sorted_files.sort_unstable_by(|a, b| a.0.cmp(&b.0));

    let mut status_lines = Vec::new();
    if let Some(ref volatile_path) = options.volatile_status_file {
        let volatile_text = read_text_universal_newlines(volatile_path)?;
        status_lines.extend(volatile_text.lines().map(ToString::to_string));
    }
    if let Some(ref stable_path) = options.stable_status_file {
        let stable_text = read_text_universal_newlines(stable_path)?;
        status_lines.extend(stable_text.lines().map(ToString::to_string));
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
    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let out_file = BufWriter::with_capacity(BLOCK_SIZE, File::create(&out_path)?);
    let mut whl = maker.open_whlfile(out_file);

    for (package_filename, real_filename) in &sorted_files {
        whl.add_file(package_filename, real_filename)?;
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

    let extra_requires_map: HashMap<String, String> =
        options.extra_requires_file.iter().cloned().collect();

    let expanded_metadata = expand_metadata_requires(&raw_metadata, &extra_requires_map)?;
    let final_metadata = maker.format_metadata(&expanded_metadata, &name, description.as_deref());
    whl.add_string(&maker.distinfo_path("METADATA"), final_metadata.as_bytes())?;

    if let Some(ref ep_path) = options.entry_points_file {
        whl.add_file(&maker.distinfo_path("entry_points.txt"), ep_path)?;
    }

    data_files.sort_unstable();
    for (filename, real_path) in &data_files {
        whl.add_file(&maker.data_path(filename), real_path)?;
    }

    extra_distinfo_file.sort_unstable();
    for (filename, real_path) in &extra_distinfo_file {
        whl.add_file(&maker.distinfo_path(filename), real_path)?;
    }

    whl.add_recordfile()?;
    whl.finish()?;

    if let Some(ref name_file_path) = options.name_file {
        if let Some(parent) = name_file_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        fs::write(name_file_path, maker.wheelname())?;
    }

    Ok(())
}

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
    pub input_file: Vec<(String, PathBuf)>,
    pub input_file_list: Vec<PathBuf>,
    pub extra_distinfo_file: Vec<(String, PathBuf)>,
    pub data_files: Vec<(String, PathBuf)>,
    pub volatile_status_file: Option<PathBuf>,
    pub stable_status_file: Option<PathBuf>,
    pub extra_requires_file: Vec<(String, String)>,
}

fn parse_target_path_pair(s: &str) -> Result<(String, PathBuf), String> {
    let (target, real_path) = s
        .split_once(';')
        .ok_or_else(|| format!("expected 'target;real_path' pair, got '{s}'"))?;
    Ok((target.to_string(), PathBuf::from(real_path)))
}

fn parse_extra_requires_pair(s: &str) -> Result<(String, String), String> {
    let (file, extra_opt) = s
        .split_once(';')
        .ok_or_else(|| format!("expected 'file;extra' pair, got '{s}'"))?;
    let trimmed_extra = extra_opt.trim();
    let formatted_extra = if trimmed_extra.starts_with("extra") {
        trimmed_extra.to_string()
    } else {
        format!("extra == '{trimmed_extra}'")
    };
    Ok((file.trim().to_string(), formatted_extra))
}

/// Parses command-line arguments (excluding `argv[0]`) into `WheelOptions`.
pub fn parse_args<I, T>(args: I) -> Result<WheelOptions, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    use lexopt::prelude::*;

    let expanded = argfile::expand_args_from(
        args.into_iter().map(Into::into),
        argfile::parse_fromfile,
        argfile::PREFIX,
    )
    .map_err(|err| format!("failed to read flagfile: {err}"))?;

    let mut opts = WheelOptions {
        python_tag: "py3".to_string(),
        abi: "none".to_string(),
        platform: "any".to_string(),
        ..Default::default()
    };
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;

    let mut parser = lexopt::Parser::from_args(expanded);
    while let Some(arg) = parser.next().map_err(|err| err.to_string())? {
        match arg {
            Short('h') | Long("help") => {
                return Err("Usage: wheelmaker --name NAME --version VERSION [OPTIONS]".to_string());
            }
            Long("name") => {
                name = Some(
                    parser
                        .value()
                        .and_then(|v| v.string())
                        .map_err(|err| err.to_string())?,
                );
            }
            Long("version") => {
                version = Some(
                    parser
                        .value()
                        .and_then(|v| v.string())
                        .map_err(|err| err.to_string())?,
                );
            }
            Long("build_tag") => {
                opts.build_tag = parser
                    .value()
                    .and_then(|v| v.string())
                    .map_err(|err| err.to_string())?;
            }
            Long("python_tag") => {
                opts.python_tag = parser
                    .value()
                    .and_then(|v| v.string())
                    .map_err(|err| err.to_string())?;
            }
            Long("abi") => {
                opts.abi = parser
                    .value()
                    .and_then(|v| v.string())
                    .map_err(|err| err.to_string())?;
            }
            Long("platform") => {
                opts.platform = parser
                    .value()
                    .and_then(|v| v.string())
                    .map_err(|err| err.to_string())?;
            }
            Long("out") => {
                opts.out = Some(PathBuf::from(
                    parser.value().map_err(|err| err.to_string())?,
                ));
            }
            Long("no_compress") => {
                opts.no_compress = true;
            }
            Long("Compress" | "compress") => {
                opts.no_compress = false;
            }
            Long("name_file") => {
                opts.name_file = Some(PathBuf::from(
                    parser.value().map_err(|err| err.to_string())?,
                ));
            }
            Long("strip_path_prefix") => {
                opts.strip_path_prefix.push(
                    parser
                        .value()
                        .and_then(|v| v.string())
                        .map_err(|err| err.to_string())?,
                );
            }
            Long("path_prefix") => {
                opts.path_prefix = parser
                    .value()
                    .and_then(|v| v.string())
                    .map_err(|err| err.to_string())?;
            }
            Long("metadata_file") => {
                opts.metadata_file = Some(PathBuf::from(
                    parser.value().map_err(|err| err.to_string())?,
                ));
            }
            Long("description_file") => {
                opts.description_file = Some(PathBuf::from(
                    parser.value().map_err(|err| err.to_string())?,
                ));
            }
            Long("description_content_type") => {
                opts.description_content_type = Some(
                    parser
                        .value()
                        .and_then(|v| v.string())
                        .map_err(|err| err.to_string())?,
                );
            }
            Long("entry_points_file") => {
                opts.entry_points_file = Some(PathBuf::from(
                    parser.value().map_err(|err| err.to_string())?,
                ));
            }
            Long("input_file") => {
                let pair = parser
                    .value()
                    .and_then(|v| v.parse_with(parse_target_path_pair))
                    .map_err(|err| err.to_string())?;
                opts.input_file.push(pair);
            }
            Long("input_file_list") => {
                opts.input_file_list.push(PathBuf::from(
                    parser.value().map_err(|err| err.to_string())?,
                ));
            }
            Long("extra_distinfo_file") => {
                let pair = parser
                    .value()
                    .and_then(|v| v.parse_with(parse_target_path_pair))
                    .map_err(|err| err.to_string())?;
                opts.extra_distinfo_file.push(pair);
            }
            Long("data_files") => {
                let pair = parser
                    .value()
                    .and_then(|v| v.parse_with(parse_target_path_pair))
                    .map_err(|err| err.to_string())?;
                opts.data_files.push(pair);
            }
            Long("volatile_status_file") => {
                opts.volatile_status_file = Some(PathBuf::from(
                    parser.value().map_err(|err| err.to_string())?,
                ));
            }
            Long("stable_status_file") => {
                opts.stable_status_file = Some(PathBuf::from(
                    parser.value().map_err(|err| err.to_string())?,
                ));
            }
            Long("extra_requires_file") => {
                let pair = parser
                    .value()
                    .and_then(|v| v.parse_with(parse_extra_requires_pair))
                    .map_err(|err| err.to_string())?;
                opts.extra_requires_file.push(pair);
            }
            Value(val) if val.is_empty() => {}
            _ => return Err(arg.unexpected().to_string()),
        }
    }

    opts.name = name.ok_or_else(|| "missing required argument: --name".to_string())?;
    opts.version = version.ok_or_else(|| "missing required argument: --version".to_string())?;
    Ok(opts)
}

/// Parses CLI arguments and builds the wheel.
pub fn run<I, T>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = parse_args(args)?;
    build_wheel(&options).map_err(|err| err.to_string())
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), builds the wheel, and returns
/// the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    if let Err(err) = run(args) {
        eprintln!("wheelmaker: error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
