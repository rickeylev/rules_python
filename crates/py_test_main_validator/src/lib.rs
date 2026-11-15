//! Static validator checking that a `py_test` main module runs test code.
//!
//! Mirrors `python/private/py_test_main_validator.py`: parses the Python main
//! module and fails if the module defines top-level test classes or functions
//! (`def`, `async def`, `class`) while every top-level statement is inert.

use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;

use lexopt::prelude::*;
use ruff_python_ast::visitor::{Visitor, walk_expr};
use ruff_python_ast::{ExceptHandler, Expr, ModModule, Stmt};

/// Error returned when a Python source file cannot be decoded or parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
}

impl SyntaxError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SyntaxError {}

/// Formats the validation failure message matching `_format_error` in
/// `python/private/py_test_main_validator.py`.
pub fn format_error(label: &str, src_name: &str) -> String {
    let target = if label.is_empty() { src_name } else { label };
    format!(
        "py_test target {target} will not run any tests.\n\
         \n\
         The main module '{src_name}' only contains inert top-level statements \
         (class/function definitions, imports, assignments). Running it does \
         nothing, so the test silently passes without executing any test \
         code.\n\
         \n\
         py_test runs the main module directly; it does not automatically \
         invoke a test runner such as unittest or pytest. Add code that runs \
         your tests, for example:\n\
         \n\
         \x20   if __name__ == \"__main__\":\n\
         \x20       unittest.main()\n\
         \n\
         or use a main module that invokes a runner (e.g. pytest.main()).\n\
         \n\
         This check can be disabled by setting \
         --@rules_python//python/config_settings:validate_test_main=disabled."
    )
}

fn is_blank_or_comment_line(line: &[u8]) -> bool {
    let first_non_ws = line
        .iter()
        .copied()
        .find(|&b| !matches!(b, b' ' | b'\t' | b'\x0c'));
    matches!(first_non_ws, None | Some(b'#'))
}

fn extract_pep263_encoding(body: &[u8]) -> Option<&str> {
    static COOKIE_RE: OnceLock<regex::bytes::Regex> = OnceLock::new();
    let re = COOKIE_RE.get_or_init(|| {
        regex::bytes::Regex::new(r"(?-u)^[ \t\x0c]*#.*?coding[:=][ \t]*([-_.a-zA-Z0-9]+)")
            .expect("valid PEP 263 regex")
    });

    let mut lines = body.split(|&b| b == b'\n');
    let line1 = lines.next()?;
    let line1 = line1.strip_suffix(b"\r").unwrap_or(line1);
    if let Some(caps) = re.captures(line1) {
        return std::str::from_utf8(caps.get(1)?.as_bytes()).ok();
    }
    if !is_blank_or_comment_line(line1) {
        return None;
    }

    let line2 = lines.next()?;
    let line2 = line2.strip_suffix(b"\r").unwrap_or(line2);
    let caps = re.captures(line2)?;
    std::str::from_utf8(caps.get(1)?.as_bytes()).ok()
}

fn decode_python_source(source: &[u8]) -> Result<Cow<'_, str>, SyntaxError> {
    if source.contains(&0) {
        return Err(SyntaxError::new("source code cannot contain null bytes"));
    }

    let (has_bom, body) = match source.strip_prefix(b"\xef\xbb\xbf") {
        Some(rest) => (true, rest),
        None => (false, source),
    };

    let Some(raw_enc) = extract_pep263_encoding(body) else {
        return std::str::from_utf8(body)
            .map(Cow::Borrowed)
            .map_err(|_| SyntaxError::new("invalid utf-8 encoding"));
    };

    let normalized = raw_enc.to_ascii_lowercase().replace('_', "-");
    let is_utf8 = matches!(normalized.as_str(), "utf-8" | "utf8" | "utf-8-sig");
    if has_bom && !is_utf8 {
        return Err(SyntaxError::new("encoding problem: utf-8 with BOM"));
    }

    if is_utf8 {
        return std::str::from_utf8(body)
            .map(Cow::Borrowed)
            .map_err(|_| SyntaxError::new("invalid utf-8 encoding"));
    }

    match normalized.as_str() {
        "ascii" | "us-ascii" | "646" | "ansi-x3.4-1968" => {
            if !body.is_ascii() {
                return Err(SyntaxError::new("'ascii' codec can't decode byte"));
            }
            std::str::from_utf8(body)
                .map(Cow::Borrowed)
                .map_err(|_| SyntaxError::new("invalid ascii encoding"))
        }
        "latin-1" | "latin1" | "iso-8859-1" | "iso8859-1" | "iso-latin-1" | "latin" | "l1" => {
            if body.is_ascii()
                && let Ok(ascii_str) = std::str::from_utf8(body)
            {
                Ok(Cow::Borrowed(ascii_str))
            } else {
                Ok(Cow::Owned(body.iter().map(|&b| char::from(b)).collect()))
            }
        }
        _ => Err(SyntaxError::new(format!("unknown encoding: {raw_enc}"))),
    }
}

#[derive(Default)]
struct ActiveExprFinder {
    found: bool,
}

impl<'a> Visitor<'a> for ActiveExprFinder {
    fn visit_expr(&mut self, expr: &'a Expr) {
        if self.found {
            return;
        }
        if matches!(
            expr,
            Expr::Call(_) | Expr::Await(_) | Expr::Yield(_) | Expr::YieldFrom(_)
        ) {
            self.found = true;
            return;
        }
        walk_expr(self, expr);
    }
}

fn expr_runs_code(expr: &Expr) -> bool {
    let mut finder = ActiveExprFinder::default();
    finder.visit_expr(expr);
    finder.found
}

fn all_inert(stmts: &[Stmt]) -> bool {
    stmts.iter().all(is_inert_statement)
}

/// Returns `true` if `stmt` does not run any test code.
pub fn is_inert_statement(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::FunctionDef(_)
        | Stmt::ClassDef(_)
        | Stmt::Import(_)
        | Stmt::ImportFrom(_)
        | Stmt::Global(_)
        | Stmt::Pass(_)
        | Stmt::TypeAlias(_) => true,
        Stmt::Assign(node) => !expr_runs_code(&node.value),
        Stmt::AugAssign(node) => !expr_runs_code(&node.value),
        Stmt::AnnAssign(node) => node.value.as_deref().is_none_or(|v| !expr_runs_code(v)),
        Stmt::Assert(node) => {
            !expr_runs_code(&node.test) && node.msg.as_deref().is_none_or(|m| !expr_runs_code(m))
        }
        Stmt::Expr(node) => !expr_runs_code(&node.value),
        Stmt::If(node) => {
            !expr_runs_code(&node.test)
                && all_inert(&node.body)
                && node.elif_else_clauses.iter().all(|clause| {
                    clause.test.as_ref().is_none_or(|t| !expr_runs_code(t))
                        && all_inert(&clause.body)
                })
        }
        Stmt::Try(node) => {
            all_inert(&node.body)
                && all_inert(&node.orelse)
                && all_inert(&node.finalbody)
                && node
                    .handlers
                    .iter()
                    .all(|ExceptHandler::ExceptHandler(h)| all_inert(&h.body))
        }
        _ => false,
    }
}

/// Returns `true` if the module body has at least one active statement.
pub fn module_runs_something(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| !is_inert_statement(stmt))
}

/// Returns `true` if the module defines any top-level class or function.
pub fn defines_test_code(body: &[Stmt]) -> bool {
    body.iter()
        .any(|stmt| matches!(stmt, Stmt::FunctionDef(_) | Stmt::ClassDef(_)))
}

/// Returns `true` if the module appears able to run tests (or defines no test code).
pub fn module_runs_tests(body: &[Stmt]) -> bool {
    module_runs_something(body) || !defines_test_code(body)
}

fn parse_python_module(source: &str) -> Result<ModModule, SyntaxError> {
    if source.contains('\0') {
        return Err(SyntaxError::new("source code cannot contain null bytes"));
    }
    ruff_python_parser::parse_module(source)
        .map(ruff_python_parser::Parsed::into_syntax)
        .map_err(|e| SyntaxError::new(e.to_string()))
}

/// Parses `source` bytes and returns whether the module runs tests (or defines no test code).
pub fn module_runs_tests_bytes(source: &[u8]) -> Result<bool, SyntaxError> {
    let text = decode_python_source(source)?;
    module_runs_tests_str(&text)
}

/// Parses `source` string and returns whether the module runs tests (or defines no test code).
pub fn module_runs_tests_str(source: &str) -> Result<bool, SyntaxError> {
    let module = parse_python_module(source)?;
    Ok(module_runs_tests(&module.body))
}

/// Parses `source` string and returns whether the module has at least one active top-level
/// statement.
pub fn module_runs_something_str(source: &str) -> Result<bool, SyntaxError> {
    let module = parse_python_module(source)?;
    Ok(module_runs_something(&module.body))
}

/// CLI options for `py_test_main_validator`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorOptions {
    pub src: PathBuf,
    pub src_name: String,
    pub label: String,
    pub output: PathBuf,
}

/// Parses CLI arguments (excluding `argv[0]`) into `ValidatorOptions`.
pub fn parse_args<I, T>(args: I) -> Result<ValidatorOptions, String>
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

    let mut src: Option<PathBuf> = None;
    let mut src_name = String::new();
    let mut label = String::new();
    let mut output: Option<PathBuf> = None;

    let mut parser = lexopt::Parser::from_args(expanded);
    while let Some(arg) = parser.next().map_err(|e| e.to_string())? {
        match arg {
            Short('h') | Long("help") => {
                return Err(
                    "Usage: py_test_main_validator --src <path> [--src_name <name>] \
                     [--label <label>] --output <path>"
                        .to_string(),
                );
            }
            Long("src") => {
                let val = parser.value().map_err(|e| e.to_string())?;
                src = Some(PathBuf::from(val));
            }
            Long("src_name") => {
                src_name = parser
                    .value()
                    .map_err(|e| e.to_string())?
                    .string()
                    .map_err(|e| e.to_string())?;
            }
            Long("label") => {
                label = parser
                    .value()
                    .map_err(|e| e.to_string())?
                    .string()
                    .map_err(|e| e.to_string())?;
            }
            Long("output") => {
                let val = parser.value().map_err(|e| e.to_string())?;
                output = Some(PathBuf::from(val));
            }
            _ => return Err(arg.unexpected().to_string()),
        }
    }

    let src = src.ok_or_else(|| "the following arguments are required: --src".to_string())?;
    let output =
        output.ok_or_else(|| "the following arguments are required: --output".to_string())?;

    if src_name.is_empty() {
        src_name = src.display().to_string();
    }

    Ok(ValidatorOptions {
        src,
        src_name,
        label,
        output,
    })
}

fn write_empty_output(output: &Path) -> io::Result<()> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, "")
}

/// Runs validation according to `options`, writing any diagnostic messages to `stderr`.
pub fn validate_test_main(
    options: &ValidatorOptions,
    stderr: &mut dyn Write,
) -> io::Result<ExitCode> {
    let source = fs::read(&options.src)?;
    let fallback_name;
    let src_name = if options.src_name.is_empty() {
        fallback_name = options.src.display().to_string();
        fallback_name.as_str()
    } else {
        options.src_name.as_str()
    };

    match module_runs_tests_bytes(&source) {
        Err(err) => {
            writeln!(
                stderr,
                "WARNING: py_test main validator could not parse {src_name}: {err}"
            )?;
            write_empty_output(&options.output)?;
            Ok(ExitCode::SUCCESS)
        }
        Ok(false) => {
            writeln!(stderr, "{}", format_error(&options.label, src_name))?;
            Ok(ExitCode::FAILURE)
        }
        Ok(true) => {
            write_empty_output(&options.output)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Top-level CLI runner taking raw arguments (excluding `argv[0]`).
pub fn run<I, T>(args: I, stderr: &mut dyn Write) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let options = match parse_args(args) {
        Ok(opts) => opts,
        Err(msg) => {
            let _ = writeln!(stderr, "py_test_main_validator: error: {msg}");
            return ExitCode::from(2);
        }
    };

    match validate_test_main(&options, stderr) {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(stderr, "py_test_main_validator: error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), validates the test main
/// module, and returns the process exit code.
pub fn run_cli<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    run(args, &mut io::stderr())
}
