//! Library for generating console_script entrypoint Python files from `entry_points.txt`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

/// Parsed command-line options for `py_console_script_gen`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub entry_points: PathBuf,
    pub out: PathBuf,
    pub console_script: Option<String>,
    pub console_script_guess: String,
    pub shebang: String,
}

/// Expands `@flagfile` arguments recursively/iteratively.
fn expand_args<I, S>(args: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut stack: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
    stack.reverse();
    let mut expanded = Vec::new();

    while let Some(arg) = stack.pop() {
        if let Some(file_path) = arg.strip_prefix('@') {
            let content = fs::read_to_string(file_path)
                .map_err(|e| format!("failed to read flagfile '{}': {}", file_path, e))?;
            for line in content.lines().rev() {
                stack.push(line.to_string());
            }
        } else {
            expanded.push(arg);
        }
    }

    Ok(expanded)
}

/// Parses command-line arguments (excluding `argv[0]`) into `Options`.
pub fn parse_args<I, S>(args: I) -> Result<Options, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let expanded = expand_args(args)?;
    let mut positional = Vec::new();
    let mut console_script: Option<String> = None;
    let mut console_script_guess: Option<String> = None;
    let mut shebang = String::new();
    let mut passthrough = false;

    let mut iter = expanded.into_iter();
    while let Some(arg) = iter.next() {
        if passthrough {
            positional.push(arg);
            continue;
        }
        if arg == "--" {
            passthrough = true;
            continue;
        }
        if arg == "-h" || arg == "--help" {
            return Err(
                "Usage: py_console_script_gen [--console-script <str>] --console-script-guess <str> [--shebang <str>] <ENTRY_POINTS_TXT> <OUT>"
                    .to_string(),
            );
        } else if let Some(val) = arg.strip_prefix("--console-script=") {
            console_script = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            };
        } else if arg == "--console-script" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --console-script".to_string())?;
            console_script = if val.is_empty() { None } else { Some(val) };
        } else if let Some(val) = arg.strip_prefix("--console-script-guess=") {
            console_script_guess = Some(val.to_string());
        } else if arg == "--console-script-guess" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --console-script-guess".to_string())?;
            console_script_guess = Some(val);
        } else if let Some(val) = arg.strip_prefix("--shebang=") {
            shebang = val.to_string();
        } else if arg == "--shebang" {
            let val = iter
                .next()
                .ok_or_else(|| "missing value for --shebang".to_string())?;
            shebang = val;
        } else if arg.starts_with('-') {
            return Err(format!("unknown option: {}", arg));
        } else {
            positional.push(arg);
        }
    }

    let console_script_guess = console_script_guess.ok_or_else(|| {
        "the following arguments are required: --console-script-guess".to_string()
    })?;

    if positional.len() != 2 {
        return Err(format!(
            "expected 2 positional arguments (ENTRY_POINTS_TXT OUT), got {}",
            positional.len()
        ));
    }

    Ok(Options {
        entry_points: PathBuf::from(&positional[0]),
        out: PathBuf::from(&positional[1]),
        console_script,
        console_script_guess,
        shebang,
    })
}

/// Parses the `[console_scripts]` section from an `entry_points.txt` INI string.
///
/// Preserves case-sensitivity of keys (`optionxform = str`).
/// Returns `None` if `[console_scripts]` is absent or empty.
pub fn parse_console_scripts(content: &str) -> Result<Option<BTreeMap<String, String>>, String> {
    let mut in_console_scripts = false;
    let mut found_console_scripts_section = false;
    let mut console_scripts: BTreeMap<String, String> = BTreeMap::new();
    let mut last_key: Option<String> = None;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
            let section_name = &trimmed[1..trimmed.len() - 1];
            in_console_scripts = section_name == "console_scripts";
            if in_console_scripts {
                found_console_scripts_section = true;
            }
            last_key = None;
            continue;
        }

        let is_indented = raw_line.starts_with(' ') || raw_line.starts_with('\t');
        if is_indented {
            if in_console_scripts {
                if let Some(ref key) = last_key {
                    if let Some(existing) = console_scripts.get_mut(key) {
                        if !existing.is_empty() {
                            existing.push('\n');
                        }
                        existing.push_str(trimmed);
                        continue;
                    }
                }
            } else {
                continue;
            }
        }

        if !in_console_scripts {
            continue;
        }

        if let Some(sep_idx) = raw_line.find(['=', ':']) {
            let key = raw_line[..sep_idx].trim().to_string();
            let value = raw_line[sep_idx + 1..].trim().to_string();
            if !key.is_empty() {
                console_scripts.insert(key.clone(), value);
                last_key = Some(key);
            }
        }
    }

    if !found_console_scripts_section || console_scripts.is_empty() {
        return Ok(None);
    }

    Ok(Some(console_scripts))
}

/// Renders the console script template for the given module, imported attribute, entry point, and shebang.
pub fn render_template(shebang: &str, module: &str, attr: &str, entry_point: &str) -> String {
    let shebang_prefix = if shebang.is_empty() {
        String::new()
    } else {
        format!("{}\n", shebang)
    };

    format!(
        "{shebang_prefix}import sys

# See @rules_python//python/private:py_console_script_gen.py for explanation
if getattr(sys.flags, \"safe_path\", False):
    # We are running on Python 3.11 and we don't need this workaround
    pass
elif \".runfiles\" not in sys.path[0]:
    sys.path = sys.path[1:]

try:
    from {module} import {attr}
except ImportError:
    entries = \"\\n\".join(sys.path)
    print(\"Printing sys.path entries for easier debugging:\", file=sys.stderr)
    print(f\"sys.path is:\\n{{entries}}\", file=sys.stderr)
    raise

if __name__ == \"__main__\":
    sys.exit({entry_point}())  # pyrefly: ignore[not-callable]
"
    )
}

/// Generates a console script Python file from `entry_points.txt` according to the given parameters.
pub fn run(
    entry_points: &Path,
    out: &Path,
    console_script: Option<&str>,
    console_script_guess: &str,
    shebang: &str,
) -> Result<(), String> {
    let content = fs::read_to_string(entry_points)
        .map_err(|e| format!("failed to read {}: {}", entry_points.display(), e))?;

    let console_scripts = parse_console_scripts(&content)?.ok_or_else(|| {
        "The package does not provide any console_scripts in its entry_points.txt".to_string()
    })?;

    let non_empty_script = console_script.filter(|s| !s.is_empty());

    let entry_point = if let Some(script_name) = non_empty_script {
        match console_scripts.get(script_name) {
            Some(ep) => ep.as_str(),
            None => {
                let available: Vec<&str> = console_scripts.keys().map(String::as_str).collect();
                return Err(format!(
                    "The console_script '{}' was not found, only the following are available: {}",
                    script_name,
                    available.join(", ")
                ));
            }
        }
    } else {
        match console_scripts
            .get(console_script_guess)
            .filter(|ep| !ep.is_empty())
        {
            Some(ep) => ep.as_str(),
            None => {
                let available: Vec<&str> = console_scripts.keys().map(String::as_str).collect();
                return Err(format!(
                    "Tried to guess that you wanted '{}', but could not find it. Please select one of the following console scripts: {}",
                    console_script_guess,
                    available.join(", ")
                ));
            }
        }
    };

    let (module, entry_point_callable) = match entry_point.rfind(':') {
        Some(idx) => (&entry_point[..idx], &entry_point[idx + 1..]),
        None => ("", entry_point),
    };

    let attr = match entry_point_callable.find('.') {
        Some(idx) => &entry_point_callable[..idx],
        None => entry_point_callable,
    };

    let rendered = render_template(shebang, module, attr, entry_point_callable);

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create directory {}: {}", parent.display(), e))?;
        }
    }

    fs::write(out, rendered).map_err(|e| format!("failed to write {}: {}", out.display(), e))?;

    Ok(())
}

/// Runs the generator using parsed `Options`.
pub fn run_with_options(options: &Options) -> Result<(), String> {
    run(
        &options.entry_points,
        &options.out,
        options.console_script.as_deref(),
        &options.console_script_guess,
        &options.shebang,
    )
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), generates the console script,
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
            eprintln!("py_console_script_gen: error: {}", msg);
            return 1;
        }
    };
    if let Err(err) = run_with_options(&options) {
        eprintln!("py_console_script_gen: error: {}", err);
        return 1;
    }
    0
}
