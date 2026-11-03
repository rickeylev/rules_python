//! Static validator checking that a `py_test` main module runs test code.
//!
//! Mirrors `python/private/py_test_main_validator.py`: parses the Python main
//! module and fails if the module defines top-level test classes or functions
//! (`def`, `async def`, `class`) while every top-level statement is inert.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// CLI options for `py_test_main_validator`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorOptions {
    pub src: PathBuf,
    pub src_name: String,
    pub label: String,
    pub output: PathBuf,
}

/// Error returned when a Python source file cannot be lexed or parsed.
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

fn expand_flagfiles<I, S>(args: I) -> Result<Vec<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut expanded = Vec::new();
    for item in args {
        let arg = item.as_ref();
        if let Some(flagfile_path) = arg.strip_prefix('@') {
            let content = fs::read_to_string(flagfile_path)
                .map_err(|e| format!("failed to read flagfile '{}': {}", flagfile_path, e))?;
            for line in content.lines() {
                expanded.push(line.to_string());
            }
        } else {
            expanded.push(arg.to_string());
        }
    }
    Ok(expanded)
}

/// Parses CLI arguments (excluding `argv[0]`) into `ValidatorOptions`.
pub fn parse_args<I, S>(args: I) -> Result<ValidatorOptions, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let expanded = expand_flagfiles(args)?;
    let mut src: Option<PathBuf> = None;
    let mut src_name = String::new();
    let mut label = String::new();
    let mut output: Option<PathBuf> = None;

    let mut iter = expanded.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "-h" || arg == "--help" {
            return Err(
                "Usage: py_test_main_validator --src <path> [--src_name <name>] [--label <label>] --output <path>"
                    .to_string(),
            );
        } else if let Some(val) = arg.strip_prefix("--src=") {
            src = Some(PathBuf::from(val));
        } else if arg == "--src" {
            let val = iter
                .next()
                .ok_or_else(|| "argument --src: expected one argument".to_string())?;
            src = Some(PathBuf::from(val));
        } else if let Some(val) = arg.strip_prefix("--src_name=") {
            src_name = val.to_string();
        } else if arg == "--src_name" {
            let val = iter
                .next()
                .ok_or_else(|| "argument --src_name: expected one argument".to_string())?;
            src_name = val;
        } else if let Some(val) = arg.strip_prefix("--label=") {
            label = val.to_string();
        } else if arg == "--label" {
            let val = iter
                .next()
                .ok_or_else(|| "argument --label: expected one argument".to_string())?;
            label = val;
        } else if let Some(val) = arg.strip_prefix("--output=") {
            output = Some(PathBuf::from(val));
        } else if arg == "--output" {
            let val = iter
                .next()
                .ok_or_else(|| "argument --output: expected one argument".to_string())?;
            output = Some(PathBuf::from(val));
        } else {
            return Err(format!("unrecognized argument: {}", arg));
        }
    }

    let src = src.ok_or_else(|| "the following arguments are required: --src".to_string())?;
    let output =
        output.ok_or_else(|| "the following arguments are required: --output".to_string())?;

    if src_name.is_empty() {
        src_name = src.to_string_lossy().into_owned();
    }

    Ok(ValidatorOptions {
        src,
        src_name,
        label,
        output,
    })
}

fn write_empty_output(output: &Path) -> io::Result<()> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(output, "")
}

/// Runs validation according to `options`, writing any diagnostic messages to `stderr`.
/// Returns the process exit code (`0` or `1`).
pub fn validate_test_main(options: &ValidatorOptions, stderr: &mut dyn Write) -> io::Result<i32> {
    let source = fs::read(&options.src)?;
    let src_name = if options.src_name.is_empty() {
        options.src.to_string_lossy().into_owned()
    } else {
        options.src_name.clone()
    };

    match module_runs_tests_bytes(&source) {
        Err(err) => {
            writeln!(
                stderr,
                "WARNING: py_test main validator could not parse {}: {}",
                src_name, err
            )?;
            write_empty_output(&options.output)?;
            Ok(0)
        }
        Ok(false) => {
            writeln!(stderr, "{}", format_error(&options.label, &src_name))?;
            Ok(1)
        }
        Ok(true) => {
            write_empty_output(&options.output)?;
            Ok(0)
        }
    }
}

/// Top-level CLI runner taking raw argument strings (excluding `argv[0]`).
pub fn run<I, S>(args: I, stderr: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let options = match parse_args(args) {
        Ok(opts) => opts,
        Err(msg) => {
            let _ = writeln!(stderr, "py_test_main_validator: error: {}", msg);
            return 2;
        }
    };

    match validate_test_main(&options, stderr) {
        Ok(code) => code,
        Err(err) => {
            let _ = writeln!(stderr, "py_test_main_validator: error: {}", err);
            1
        }
    }
}

/// Parses CLI arguments (excluding `argv[0]` and subcommand name), validates the test main
/// module, and returns the process exit code.
pub fn run_cli<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let str_args: Vec<String> = args
        .into_iter()
        .map(|a| a.into().to_string_lossy().into_owned())
        .collect();
    run(&str_args, &mut io::stderr())
}

/// Parses `source` bytes and returns whether the module runs tests (or defines no test code).
pub fn module_runs_tests_bytes(source: &[u8]) -> Result<bool, SyntaxError> {
    if source.contains(&0) {
        return Err(SyntaxError::new("source code cannot contain null bytes"));
    }
    let source = source.strip_prefix(b"\xef\xbb\xbf").unwrap_or(source);
    let text =
        std::str::from_utf8(source).map_err(|_| SyntaxError::new("invalid utf-8 encoding"))?;
    module_runs_tests_str(text)
}

/// Parses `source` string and returns whether the module runs tests (or defines no test code).
pub fn module_runs_tests_str(source: &str) -> Result<bool, SyntaxError> {
    if source.as_bytes().contains(&0) {
        return Err(SyntaxError::new("source code cannot contain null bytes"));
    }
    let stmts = parse_module(source)?;
    Ok(module_runs_tests(&stmts))
}

/// Parses `source` string and returns whether the module has at least one active top-level statement.
pub fn module_runs_something_str(source: &str) -> Result<bool, SyntaxError> {
    if source.as_bytes().contains(&0) {
        return Err(SyntaxError::new("source code cannot contain null bytes"));
    }
    let stmts = parse_module(source)?;
    Ok(module_runs_something(&stmts))
}

/// Parsed Python statement representation sufficient for inertness and test-definition analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    /// `def`, `async def`, or `class` definition.
    DefOrClass,
    /// Always-inert simple statement (`import`, `from ... import`, `global`, `pass`, `type X = ...`).
    InertSimple,
    /// `Assign`, `AnnAssign`, or `AugAssign`.
    Assign { value_runs_code: bool },
    /// `Assert` statement.
    Assert { runs_code: bool },
    /// Standalone expression statement (`Expr`).
    Expr { runs_code: bool },
    /// `If` / `elif` / `else` statement.
    If {
        cond_runs_code: bool,
        body: Vec<Stmt>,
        orelse: Vec<Stmt>,
    },
    /// `Try` / `except` / `else` / `finally` statement.
    Try {
        body: Vec<Stmt>,
        handlers: Vec<Vec<Stmt>>,
        orelse: Vec<Stmt>,
        finalbody: Vec<Stmt>,
    },
    /// Any other statement (`for`, `while`, `with`, `match`, `raise`, `del`, `return`, `break`, `continue`, `nonlocal`).
    ActiveOther,
}

fn all_inert(stmts: &[Stmt]) -> bool {
    stmts.iter().all(is_inert_statement)
}

/// Returns `true` if `stmt` does not run any test code.
pub fn is_inert_statement(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::DefOrClass | Stmt::InertSimple => true,
        Stmt::Assign { value_runs_code } => !*value_runs_code,
        Stmt::Assert { runs_code } => !*runs_code,
        Stmt::Expr { runs_code } => !*runs_code,
        Stmt::If {
            cond_runs_code,
            body,
            orelse,
        } => !*cond_runs_code && all_inert(body) && all_inert(orelse),
        Stmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        } => {
            all_inert(body)
                && all_inert(orelse)
                && all_inert(finalbody)
                && handlers.iter().all(|h| all_inert(h))
        }
        Stmt::ActiveOther => false,
    }
}

/// Returns `true` if the module body has at least one active statement.
pub fn module_runs_something(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| !is_inert_statement(stmt))
}

/// Returns `true` if the module defines any top-level class or function.
pub fn defines_test_code(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| matches!(stmt, Stmt::DefOrClass))
}

/// Returns `true` if the module appears able to run tests (or defines no test code).
pub fn module_runs_tests(body: &[Stmt]) -> bool {
    if module_runs_something(body) {
        return true;
    }
    !defines_test_code(body)
}

// ============================================================================
// Python Lexer
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Indent,
    Dedent,
    Newline,
    Ident(String),
    Number,
    StringLit { has_active_expr: bool },
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Colon,
    Comma,
    Semi,
    Dot,
    Ellipsis,
    At,
    Arrow,
    Assign,
    Walrus,
    AugAssign,
    Op(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: TokenKind,
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

fn is_string_prefix(s: &str) -> Option<bool> {
    // Returns Some(is_fstring) if `s` is a valid Python string prefix, else None.
    match s.to_ascii_lowercase().as_str() {
        "" | "r" | "u" | "b" | "br" | "rb" => Some(false),
        "f" | "fr" | "rf" => Some(true),
        _ => None,
    }
}

fn tokenize(source: &str) -> Result<Vec<Token>, SyntaxError> {
    let chars: Vec<char> = source.chars().collect();
    let n = chars.len();
    let mut pos = 0;

    let mut tokens: Vec<Token> = Vec::new();
    let mut bracket_stack: Vec<char> = Vec::new();
    let mut indent_stack: Vec<String> = vec![String::new()];
    let mut at_line_start = true;
    let mut emitted_on_line = false;

    while pos < n {
        // Handle start-of-line indentation when outside brackets.
        if at_line_start && bracket_stack.is_empty() {
            let mut ws = String::new();
            while pos < n && (chars[pos] == ' ' || chars[pos] == '\t' || chars[pos] == '\x0c') {
                if chars[pos] == '\x0c' {
                    ws.clear();
                } else {
                    ws.push(chars[pos]);
                }
                pos += 1;
            }

            // Skip blank lines and comment-only lines without changing indentation.
            if pos >= n {
                break;
            }
            if chars[pos] == '#' {
                while pos < n && chars[pos] != '\n' && chars[pos] != '\r' {
                    pos += 1;
                }
                if pos < n {
                    if chars[pos] == '\r' && pos + 1 < n && chars[pos + 1] == '\n' {
                        pos += 2;
                    } else {
                        pos += 1;
                    }
                }
                continue;
            }
            if chars[pos] == '\n' || chars[pos] == '\r' {
                if chars[pos] == '\r' && pos + 1 < n && chars[pos + 1] == '\n' {
                    pos += 2;
                } else {
                    pos += 1;
                }
                continue;
            }

            let current_indent = indent_stack.last().unwrap();
            if &ws == current_indent {
                // Same indentation level.
            } else if ws.len() > current_indent.len() {
                if !ws.starts_with(current_indent.as_str()) {
                    return Err(SyntaxError::new(
                        "inconsistent use of tabs and spaces in indentation",
                    ));
                }
                indent_stack.push(ws);
                tokens.push(Token {
                    kind: TokenKind::Indent,
                });
            } else {
                while indent_stack.last().unwrap().len() > ws.len() {
                    let prev = indent_stack.pop().unwrap();
                    if !prev.starts_with(ws.as_str()) {
                        return Err(SyntaxError::new(
                            "inconsistent use of tabs and spaces in indentation",
                        ));
                    }
                    tokens.push(Token {
                        kind: TokenKind::Dedent,
                    });
                }
                if indent_stack.last().unwrap() != &ws {
                    return Err(SyntaxError::new(
                        "unindent does not match any outer indentation level",
                    ));
                }
            }

            at_line_start = false;
            emitted_on_line = false;
        }

        if pos >= n {
            break;
        }

        let c = chars[pos];

        // Whitespace within a line.
        if c == ' ' || c == '\t' || c == '\x0c' {
            pos += 1;
            continue;
        }

        // Comment.
        if c == '#' {
            while pos < n && chars[pos] != '\n' && chars[pos] != '\r' {
                pos += 1;
            }
            continue;
        }

        // Explicit line continuation `\`.
        if c == '\\' {
            pos += 1;
            if pos < n && chars[pos] == '\r' && pos + 1 < n && chars[pos + 1] == '\n' {
                pos += 2;
                continue;
            } else if pos < n && (chars[pos] == '\n' || chars[pos] == '\r') {
                pos += 1;
                continue;
            } else {
                return Err(SyntaxError::new(
                    "unexpected character after line continuation character",
                ));
            }
        }

        // Physical newline.
        if c == '\n' || c == '\r' {
            if c == '\r' && pos + 1 < n && chars[pos + 1] == '\n' {
                pos += 2;
            } else {
                pos += 1;
            }
            if bracket_stack.is_empty() {
                if emitted_on_line {
                    tokens.push(Token {
                        kind: TokenKind::Newline,
                    });
                }
                at_line_start = true;
                emitted_on_line = false;
            }
            continue;
        }

        // String literal or identifier starting at `pos`.
        if is_ident_start(c) {
            let start = pos;
            while pos < n && is_ident_continue(chars[pos]) {
                pos += 1;
            }
            let word: String = chars[start..pos].iter().collect();

            // Check if immediately followed by `'` or `"` (string prefix).
            if pos < n && (chars[pos] == '\'' || chars[pos] == '"') {
                if let Some(is_fstring) = is_string_prefix(&word) {
                    let has_active_expr = lex_string(&chars, &mut pos, is_fstring)?;
                    push_string_token(&mut tokens, has_active_expr);
                    emitted_on_line = true;
                    continue;
                } else {
                    return Err(SyntaxError::new(format!(
                        "invalid string prefix '{}'",
                        word
                    )));
                }
            }

            tokens.push(Token {
                kind: TokenKind::Ident(word),
            });
            emitted_on_line = true;
            continue;
        }

        // Unprefixed string literal.
        if c == '\'' || c == '"' {
            let has_active_expr = lex_string(&chars, &mut pos, false)?;
            push_string_token(&mut tokens, has_active_expr);
            emitted_on_line = true;
            continue;
        }

        // Numeric literal (starting with digit, or `.` followed by digit).
        if c.is_ascii_digit() || (c == '.' && pos + 1 < n && chars[pos + 1].is_ascii_digit()) {
            lex_number(&chars, &mut pos)?;
            tokens.push(Token {
                kind: TokenKind::Number,
            });
            emitted_on_line = true;
            continue;
        }

        // Brackets.
        if c == '(' || c == '[' || c == '{' {
            bracket_stack.push(c);
            let kind = match c {
                '(' => TokenKind::LParen,
                '[' => TokenKind::LBracket,
                '{' => TokenKind::LBrace,
                _ => unreachable!(),
            };
            tokens.push(Token { kind });
            pos += 1;
            emitted_on_line = true;
            continue;
        }

        if c == ')' || c == ']' || c == '}' {
            let expected_open = match c {
                ')' => '(',
                ']' => '[',
                '}' => '{',
                _ => unreachable!(),
            };
            match bracket_stack.pop() {
                Some(open) if open == expected_open => {}
                _ => {
                    return Err(SyntaxError::new(format!(
                        "unmatched closing bracket '{}'",
                        c
                    )))
                }
            }
            let kind = match c {
                ')' => TokenKind::RParen,
                ']' => TokenKind::RBracket,
                '}' => TokenKind::RBrace,
                _ => unreachable!(),
            };
            tokens.push(Token { kind });
            pos += 1;
            emitted_on_line = true;
            continue;
        }

        // Three-character operators / tokens.
        if pos + 2 < n {
            let s3: String = chars[pos..pos + 3].iter().collect();
            let kind3 = match s3.as_str() {
                "..." => Some(TokenKind::Ellipsis),
                "//=" | "**=" | "<<=" | ">>=" => Some(TokenKind::AugAssign),
                _ => None,
            };
            if let Some(kind) = kind3 {
                tokens.push(Token { kind });
                pos += 3;
                emitted_on_line = true;
                continue;
            }
        }

        // Two-character operators / tokens.
        if pos + 1 < n {
            let s2: String = chars[pos..pos + 2].iter().collect();
            let kind2 = match s2.as_str() {
                "->" => Some(TokenKind::Arrow),
                ":=" => Some(TokenKind::Walrus),
                "+=" | "-=" | "*=" | "@=" | "/=" | "%=" | "&=" | "|=" | "^=" => {
                    Some(TokenKind::AugAssign)
                }
                "==" => Some(TokenKind::Op("==")),
                "!=" => Some(TokenKind::Op("!=")),
                "<=" => Some(TokenKind::Op("<=")),
                ">=" => Some(TokenKind::Op(">=")),
                "<<" => Some(TokenKind::Op("<<")),
                ">>" => Some(TokenKind::Op(">>")),
                "**" => Some(TokenKind::Op("**")),
                "//" => Some(TokenKind::Op("//")),
                _ => None,
            };
            if let Some(kind) = kind2 {
                tokens.push(Token { kind });
                pos += 2;
                emitted_on_line = true;
                continue;
            }
        }

        // Single-character operators / delimiters.
        let kind1 = match c {
            ':' => Some(TokenKind::Colon),
            ',' => Some(TokenKind::Comma),
            ';' => Some(TokenKind::Semi),
            '.' => Some(TokenKind::Dot),
            '=' => Some(TokenKind::Assign),
            '@' => Some(TokenKind::At),
            '+' => Some(TokenKind::Op("+")),
            '-' => Some(TokenKind::Op("-")),
            '*' => Some(TokenKind::Op("*")),
            '/' => Some(TokenKind::Op("/")),
            '%' => Some(TokenKind::Op("%")),
            '&' => Some(TokenKind::Op("&")),
            '|' => Some(TokenKind::Op("|")),
            '^' => Some(TokenKind::Op("^")),
            '~' => Some(TokenKind::Op("~")),
            '<' => Some(TokenKind::Op("<")),
            '>' => Some(TokenKind::Op(">")),
            _ => None,
        };

        if let Some(kind) = kind1 {
            tokens.push(Token { kind });
            pos += 1;
            emitted_on_line = true;
            continue;
        }

        return Err(SyntaxError::new(format!("invalid character '{}'", c)));
    }

    if !bracket_stack.is_empty() {
        return Err(SyntaxError::new("unclosed bracket at end of input"));
    }

    if emitted_on_line {
        tokens.push(Token {
            kind: TokenKind::Newline,
        });
    }

    while indent_stack.len() > 1 {
        indent_stack.pop();
        tokens.push(Token {
            kind: TokenKind::Dedent,
        });
    }

    Ok(tokens)
}

fn push_string_token(tokens: &mut Vec<Token>, has_active_expr: bool) {
    // Adjacent string literals in Python concatenate into a single string literal.
    if let Some(Token {
        kind: TokenKind::StringLit {
            has_active_expr: prev_active,
        },
    }) = tokens.last_mut()
    {
        *prev_active = *prev_active || has_active_expr;
    } else {
        tokens.push(Token {
            kind: TokenKind::StringLit { has_active_expr },
        });
    }
}

fn lex_string(chars: &[char], pos: &mut usize, is_fstring: bool) -> Result<bool, SyntaxError> {
    let n = chars.len();
    let quote = chars[*pos];
    let is_triple = *pos + 2 < n && chars[*pos + 1] == quote && chars[*pos + 2] == quote;
    if is_triple {
        *pos += 3;
    } else {
        *pos += 1;
    }

    let mut content = String::new();
    let mut closed = false;

    while *pos < n {
        let c = chars[*pos];
        if c == '\\' {
            content.push(c);
            *pos += 1;
            if *pos >= n {
                return Err(SyntaxError::new("unterminated string literal"));
            }
            content.push(chars[*pos]);
            *pos += 1;
            continue;
        }

        if is_triple {
            if *pos + 2 < n
                && chars[*pos] == quote
                && chars[*pos + 1] == quote
                && chars[*pos + 2] == quote
            {
                *pos += 3;
                closed = true;
                break;
            }
        } else if c == quote {
            *pos += 1;
            closed = true;
            break;
        } else if c == '\n' || c == '\r' {
            return Err(SyntaxError::new("unterminated string literal"));
        }

        content.push(c);
        *pos += 1;
    }

    if !closed {
        return Err(SyntaxError::new("unterminated string literal"));
    }

    if is_fstring {
        check_fstring_content(&content)
    } else {
        Ok(false)
    }
}

fn check_fstring_content(content: &str) -> Result<bool, SyntaxError> {
    let chars: Vec<char> = content.chars().collect();
    let n = chars.len();
    let mut i = 0;
    let mut runs_code = false;

    while i < n {
        if chars[i] == '\\' {
            if chars.get(i + 1) == Some(&'\\') {
                i += 2;
                continue;
            }
            if chars.get(i + 1) == Some(&'N') && chars.get(i + 2) == Some(&'{') {
                i += 3;
                while i < n && chars[i] != '}' {
                    i += 1;
                }
                if i >= n {
                    return Err(SyntaxError::new("f-string: malformed \\N character escape"));
                }
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }
        if chars[i] == '{' {
            if i + 1 < n && chars[i + 1] == '{' {
                i += 2;
                continue;
            }
            i += 1;
            if scan_fstring_field(&chars, &mut i)? {
                runs_code = true;
            }
        } else if chars[i] == '}' {
            if i + 1 < n && chars[i + 1] == '}' {
                i += 2;
                continue;
            }
            return Err(SyntaxError::new("f-string: single '}' is not allowed"));
        } else {
            i += 1;
        }
    }

    Ok(runs_code)
}

fn scan_fstring_field(chars: &[char], i: &mut usize) -> Result<bool, SyntaxError> {
    let n = chars.len();
    let expr_start = *i;
    let mut depth = 0usize;
    let mut expr_end = None;
    let mut has_format_spec = false;

    while *i < n {
        let c = chars[*i];
        if c == '\'' || c == '"' {
            // Skip nested string inside replacement field.
            let quote = c;
            let triple = *i + 2 < n && chars[*i + 1] == quote && chars[*i + 2] == quote;
            *i += if triple { 3 } else { 1 };
            let mut closed = false;
            while *i < n {
                if chars[*i] == '\\' {
                    *i += 2;
                    continue;
                }
                if triple {
                    if *i + 2 < n
                        && chars[*i] == quote
                        && chars[*i + 1] == quote
                        && chars[*i + 2] == quote
                    {
                        *i += 3;
                        closed = true;
                        break;
                    }
                } else if chars[*i] == quote {
                    *i += 1;
                    closed = true;
                    break;
                }
                *i += 1;
            }
            if !closed {
                return Err(SyntaxError::new("unterminated string in f-string"));
            }
            continue;
        }

        if c == '(' || c == '[' || c == '{' {
            depth += 1;
            *i += 1;
            continue;
        }
        if c == ')' || c == ']' {
            if depth == 0 {
                return Err(SyntaxError::new("unmatched bracket in f-string"));
            }
            depth -= 1;
            *i += 1;
            continue;
        }
        if c == '}' {
            if depth > 0 {
                depth -= 1;
                *i += 1;
                continue;
            }
            if expr_end.is_none() {
                expr_end = Some(*i);
            }
            *i += 1;
            break;
        }
        if depth == 0 {
            // Conversion `!r`, `!s`, `!a` (but not `!=`).
            if c == '!' && !(*i + 1 < n && chars[*i + 1] == '=') {
                if expr_end.is_none() {
                    expr_end = Some(*i);
                }
                *i += 1;
                if *i < n && (chars[*i] == 'r' || chars[*i] == 's' || chars[*i] == 'a') {
                    *i += 1;
                } else {
                    return Err(SyntaxError::new("invalid conversion character in f-string"));
                }
                continue;
            }
            // Format spec `:` (but not `:=`).
            if c == ':' && !(*i + 1 < n && chars[*i + 1] == '=') {
                if expr_end.is_none() {
                    expr_end = Some(*i);
                }
                *i += 1;
                has_format_spec = true;
                break;
            }
        }
        *i += 1;
    }

    let expr_end = expr_end.ok_or_else(|| SyntaxError::new("f-string: Expecting '}'"))?;

    let mut expr_slice: String = chars[expr_start..expr_end].iter().collect();
    let trimmed = expr_slice.trim_end();
    // Handle debug specifier `f"{x=}"` (but not `==`, `!=`, `<=`, `>=`, `:=`).
    if let Some(before_eq) = trimmed.strip_suffix('=') {
        if !before_eq.ends_with(['=', '!', '<', '>', ':']) {
            expr_slice = before_eq.to_string();
        }
    }

    if expr_slice.trim().is_empty() {
        return Err(SyntaxError::new("f-string: empty expression not allowed"));
    }

    let field_tokens = tokenize(&expr_slice)?;
    let expr_tokens: Vec<Token> = field_tokens
        .into_iter()
        .filter(|t| {
            !matches!(
                t.kind,
                TokenKind::Newline | TokenKind::Indent | TokenKind::Dedent
            )
        })
        .collect();
    let mut runs_code = check_expr(&expr_tokens, ExprContext::Top)?;

    if has_format_spec {
        let mut closed = false;
        while *i < n {
            if chars[*i] == '{' {
                *i += 1;
                if scan_fstring_field(chars, i)? {
                    runs_code = true;
                }
            } else if chars[*i] == '}' {
                *i += 1;
                closed = true;
                break;
            } else {
                *i += 1;
            }
        }
        if !closed {
            return Err(SyntaxError::new("f-string: Expecting '}'"));
        }
    }

    Ok(runs_code)
}

fn lex_number(chars: &[char], pos: &mut usize) -> Result<(), SyntaxError> {
    let n = chars.len();
    let start = *pos;

    if chars[*pos] == '0' && *pos + 1 < n {
        let next = chars[*pos + 1].to_ascii_lowercase();
        if next == 'x' || next == 'o' || next == 'b' {
            *pos += 2;
            let mut has_digit = false;
            let mut prev_underscore = false;
            while *pos < n {
                let c = chars[*pos];
                if c == '_' {
                    if prev_underscore {
                        return Err(SyntaxError::new("invalid numeric literal"));
                    }
                    prev_underscore = true;
                    *pos += 1;
                    continue;
                }
                let valid = match next {
                    'x' => c.is_ascii_hexdigit(),
                    'o' => ('0'..='7').contains(&c),
                    'b' => c == '0' || c == '1',
                    _ => false,
                };
                if valid {
                    has_digit = true;
                    prev_underscore = false;
                    *pos += 1;
                } else {
                    break;
                }
            }
            if !has_digit || prev_underscore {
                return Err(SyntaxError::new("invalid numeric literal"));
            }
            if *pos < n && is_ident_start(chars[*pos]) {
                return Err(SyntaxError::new("invalid numeric literal"));
            }
            return Ok(());
        }
    }

    let mut prev_underscore = false;
    while *pos < n && (chars[*pos].is_ascii_digit() || chars[*pos] == '_') {
        if chars[*pos] == '_' {
            if prev_underscore || *pos == start {
                return Err(SyntaxError::new("invalid numeric literal"));
            }
            prev_underscore = true;
        } else {
            prev_underscore = false;
        }
        *pos += 1;
    }
    if prev_underscore {
        return Err(SyntaxError::new("invalid numeric literal"));
    }

    if *pos < n && chars[*pos] == '.' {
        *pos += 1;
        while *pos < n && (chars[*pos].is_ascii_digit() || chars[*pos] == '_') {
            if chars[*pos] == '_' {
                if prev_underscore || chars[*pos - 1] == '.' {
                    return Err(SyntaxError::new("invalid numeric literal"));
                }
                prev_underscore = true;
            } else {
                prev_underscore = false;
            }
            *pos += 1;
        }
        if prev_underscore {
            return Err(SyntaxError::new("invalid numeric literal"));
        }
    }

    if *pos < n && (chars[*pos] == 'e' || chars[*pos] == 'E') {
        *pos += 1;
        if *pos < n && (chars[*pos] == '+' || chars[*pos] == '-') {
            *pos += 1;
        }
        let exp_start = *pos;
        while *pos < n && (chars[*pos].is_ascii_digit() || chars[*pos] == '_') {
            if chars[*pos] == '_' {
                if prev_underscore || *pos == exp_start {
                    return Err(SyntaxError::new("invalid numeric literal"));
                }
                prev_underscore = true;
            } else {
                prev_underscore = false;
            }
            *pos += 1;
        }
        if *pos == exp_start || prev_underscore {
            return Err(SyntaxError::new("invalid numeric literal"));
        }
    }

    if *pos < n && (chars[*pos] == 'j' || chars[*pos] == 'J') {
        *pos += 1;
    }

    if *pos < n && is_ident_start(chars[*pos]) {
        return Err(SyntaxError::new("invalid numeric literal"));
    }

    Ok(())
}

// ============================================================================
// Python Statement & Expression Parser
// ============================================================================

fn is_hard_keyword(name: &str) -> bool {
    matches!(
        name,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    )
}

fn is_atom_ident(name: &str) -> bool {
    matches!(name, "True" | "False" | "None") || !is_hard_keyword(name)
}

fn parse_module(source: &str) -> Result<Vec<Stmt>, SyntaxError> {
    let tokens = tokenize(source)?;
    let mut parser = Parser { tokens, pos: 0 };
    let mut body = Vec::new();
    while !parser.is_eof() {
        if parser.eat(&TokenKind::Newline) {
            continue;
        }
        parser.parse_stmt(&mut body)?;
    }
    Ok(body)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&TokenKind> {
        self.tokens.get(self.pos).map(|t| &t.kind)
    }

    fn peek_ident(&self, expected: &str) -> bool {
        matches!(self.peek(), Some(TokenKind::Ident(s)) if s == expected)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.peek() == Some(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_ident(&mut self, expected: &str) -> bool {
        if self.peek_ident(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_stmt(&mut self, out: &mut Vec<Stmt>) -> Result<(), SyntaxError> {
        match self.peek() {
            None => Ok(()),
            Some(TokenKind::Indent) => Err(SyntaxError::new("unexpected indent")),
            Some(TokenKind::Dedent) => Err(SyntaxError::new("unexpected dedent")),
            Some(TokenKind::At) => {
                let stmt = self.parse_decorated()?;
                out.push(stmt);
                Ok(())
            }
            Some(TokenKind::Ident(kw)) => match kw.as_str() {
                "def" => {
                    let stmt = self.parse_def()?;
                    out.push(stmt);
                    Ok(())
                }
                "class" => {
                    let stmt = self.parse_class()?;
                    out.push(stmt);
                    Ok(())
                }
                "async" => {
                    self.pos += 1;
                    if self.peek_ident("def") {
                        let stmt = self.parse_def()?;
                        out.push(stmt);
                        Ok(())
                    } else if self.peek_ident("for") {
                        let stmt = self.parse_for()?;
                        out.push(stmt);
                        Ok(())
                    } else if self.peek_ident("with") {
                        let stmt = self.parse_with()?;
                        out.push(stmt);
                        Ok(())
                    } else {
                        Err(SyntaxError::new(
                            "expected 'def', 'for', or 'with' after 'async'",
                        ))
                    }
                }
                "if" => {
                    let stmt = self.parse_if()?;
                    out.push(stmt);
                    Ok(())
                }
                "try" => {
                    let stmt = self.parse_try()?;
                    out.push(stmt);
                    Ok(())
                }
                "for" => {
                    let stmt = self.parse_for()?;
                    out.push(stmt);
                    Ok(())
                }
                "while" => {
                    let stmt = self.parse_while()?;
                    out.push(stmt);
                    Ok(())
                }
                "with" => {
                    let stmt = self.parse_with()?;
                    out.push(stmt);
                    Ok(())
                }
                "elif" | "else" | "except" | "finally" => {
                    Err(SyntaxError::new(format!("unexpected '{}'", kw)))
                }
                "match" if self.is_match_statement() => {
                    let stmt = self.parse_match()?;
                    out.push(stmt);
                    Ok(())
                }
                _ => self.parse_simple_stmt_line(out),
            },
            _ => self.parse_simple_stmt_line(out),
        }
    }

    fn parse_decorated(&mut self) -> Result<Stmt, SyntaxError> {
        while self.eat(&TokenKind::At) {
            let line_tokens = self.collect_until_newline()?;
            if line_tokens.is_empty() {
                return Err(SyntaxError::new("empty decorator expression"));
            }
            check_expr(&line_tokens, ExprContext::Top)?;
        }
        if self.peek_ident("def") {
            self.parse_def()
        } else if self.peek_ident("class") {
            self.parse_class()
        } else if self.eat_ident("async") {
            if self.peek_ident("def") {
                self.parse_def()
            } else {
                Err(SyntaxError::new("expected 'def' after '@decorator async'"))
            }
        } else {
            Err(SyntaxError::new(
                "expected 'def', 'async def', or 'class' after decorator",
            ))
        }
    }

    fn parse_def(&mut self) -> Result<Stmt, SyntaxError> {
        self.pos += 1; // consume `def`
        match self.peek() {
            Some(TokenKind::Ident(name)) if !is_hard_keyword(name) => {
                self.pos += 1;
            }
            _ => return Err(SyntaxError::new("expected function name after 'def'")),
        }

        // Optional PEP 695 type parameters `[T, ...]`.
        if self.peek() == Some(&TokenKind::LBracket) {
            let group = self.consume_balanced_group()?;
            if group.is_empty() {
                return Err(SyntaxError::new("empty type parameter list"));
            }
        }

        // Parameter list `(...)`.
        if self.peek() != Some(&TokenKind::LParen) {
            return Err(SyntaxError::new("expected '(' in function definition"));
        }
        let _params = self.consume_balanced_group()?;

        // Optional return annotation `-> expr`.
        if self.eat(&TokenKind::Arrow) {
            let ret_tokens = self.collect_header_until_colon()?;
            if ret_tokens.is_empty() {
                return Err(SyntaxError::new(
                    "expected return type annotation after '->'",
                ));
            }
            check_expr(&ret_tokens, ExprContext::Top)?;
        } else if !self.eat(&TokenKind::Colon) {
            return Err(SyntaxError::new("expected ':' in function definition"));
        }

        let _body = self.parse_suite()?;
        Ok(Stmt::DefOrClass)
    }

    fn parse_class(&mut self) -> Result<Stmt, SyntaxError> {
        self.pos += 1; // consume `class`
        match self.peek() {
            Some(TokenKind::Ident(name)) if !is_hard_keyword(name) => {
                self.pos += 1;
            }
            _ => return Err(SyntaxError::new("expected class name after 'class'")),
        }

        // Optional PEP 695 type parameters `[T, ...]`.
        if self.peek() == Some(&TokenKind::LBracket) {
            let group = self.consume_balanced_group()?;
            if group.is_empty() {
                return Err(SyntaxError::new("empty type parameter list"));
            }
        }

        // Optional base classes / keywords `(...)`.
        if self.peek() == Some(&TokenKind::LParen) {
            let bases = self.consume_balanced_group()?;
            if !bases.is_empty() {
                check_expr(&bases, ExprContext::CallArgs)?;
            }
        }

        if !self.eat(&TokenKind::Colon) {
            return Err(SyntaxError::new("expected ':' in class definition"));
        }

        let _body = self.parse_suite()?;
        Ok(Stmt::DefOrClass)
    }

    fn parse_if(&mut self) -> Result<Stmt, SyntaxError> {
        self.pos += 1; // consume `if` or `elif`
        let cond_tokens = self.collect_header_until_colon()?;
        if cond_tokens.is_empty() {
            return Err(SyntaxError::new("expected condition expression in 'if'"));
        }
        let cond_runs_code = check_expr(&cond_tokens, ExprContext::Top)?;
        let body = self.parse_suite()?;

        let orelse = if self.peek_ident("elif") {
            vec![self.parse_if()?]
        } else if self.eat_ident("else") {
            if !self.eat(&TokenKind::Colon) {
                return Err(SyntaxError::new("expected ':' after 'else'"));
            }
            self.parse_suite()?
        } else {
            Vec::new()
        };

        Ok(Stmt::If {
            cond_runs_code,
            body,
            orelse,
        })
    }

    fn parse_try(&mut self) -> Result<Stmt, SyntaxError> {
        self.pos += 1; // consume `try`
        if !self.eat(&TokenKind::Colon) {
            return Err(SyntaxError::new("expected ':' after 'try'"));
        }
        let body = self.parse_suite()?;

        let mut handlers = Vec::new();
        let mut saw_bare_except = false;
        let mut is_try_star = Option::<bool>::None;

        while self.eat_ident("except") {
            if saw_bare_except {
                return Err(SyntaxError::new("default 'except:' must be last"));
            }
            let star = self.eat(&TokenKind::Op("*"));
            if let Some(prev_star) = is_try_star {
                if prev_star != star {
                    return Err(SyntaxError::new(
                        "cannot have both 'except' and 'except*' on the same 'try'",
                    ));
                }
            } else {
                is_try_star = Some(star);
            }

            if self.eat(&TokenKind::Colon) {
                if star {
                    return Err(SyntaxError::new("expected exception type after 'except*'"));
                }
                saw_bare_except = true;
            } else {
                let header = self.collect_header_until_colon()?;
                if header.is_empty() {
                    return Err(SyntaxError::new("expected ':' in 'except' clause"));
                }
                // Check for optional `as <ident>` at end of header.
                let exc_tokens = if header.len() >= 2
                    && matches!(&header[header.len() - 2].kind, TokenKind::Ident(s) if s == "as")
                {
                    match &header[header.len() - 1].kind {
                        TokenKind::Ident(name) if !is_hard_keyword(name) => {
                            &header[..header.len() - 2]
                        }
                        _ => return Err(SyntaxError::new("invalid 'except ... as' target")),
                    }
                } else {
                    &header[..]
                };
                if exc_tokens.is_empty() {
                    return Err(SyntaxError::new("expected exception type in 'except'"));
                }
                check_expr(exc_tokens, ExprContext::Top)?;
            }
            let handler_body = self.parse_suite()?;
            handlers.push(handler_body);
        }

        let orelse = if self.eat_ident("else") {
            if handlers.is_empty() {
                return Err(SyntaxError::new(
                    "'else' without 'except' in 'try' statement",
                ));
            }
            if !self.eat(&TokenKind::Colon) {
                return Err(SyntaxError::new("expected ':' after 'else'"));
            }
            self.parse_suite()?
        } else {
            Vec::new()
        };

        let finalbody = if self.eat_ident("finally") {
            if !self.eat(&TokenKind::Colon) {
                return Err(SyntaxError::new("expected ':' after 'finally'"));
            }
            self.parse_suite()?
        } else {
            Vec::new()
        };

        if handlers.is_empty() && finalbody.is_empty() {
            return Err(SyntaxError::new("expected 'except' or 'finally' block"));
        }

        Ok(Stmt::Try {
            body,
            handlers,
            orelse,
            finalbody,
        })
    }

    fn parse_for(&mut self) -> Result<Stmt, SyntaxError> {
        self.pos += 1; // consume `for`
        let header = self.collect_header_until_colon()?;
        let in_pos = find_top_level_ident(&header, "in")
            .ok_or_else(|| SyntaxError::new("expected 'in' in 'for' loop"))?;
        let target = &header[..in_pos];
        let iter_expr = &header[in_pos + 1..];
        if target.is_empty() || iter_expr.is_empty() {
            return Err(SyntaxError::new("invalid 'for' statement header"));
        }
        validate_assign_target(target, true)?;
        check_expr(iter_expr, ExprContext::Top)?;
        let _body = self.parse_suite()?;
        if self.eat_ident("else") {
            if !self.eat(&TokenKind::Colon) {
                return Err(SyntaxError::new("expected ':' after 'else'"));
            }
            let _orelse = self.parse_suite()?;
        }
        Ok(Stmt::ActiveOther)
    }

    fn parse_while(&mut self) -> Result<Stmt, SyntaxError> {
        self.pos += 1; // consume `while`
        let cond = self.collect_header_until_colon()?;
        if cond.is_empty() {
            return Err(SyntaxError::new("expected condition in 'while' loop"));
        }
        check_expr(&cond, ExprContext::Top)?;
        let _body = self.parse_suite()?;
        if self.eat_ident("else") {
            if !self.eat(&TokenKind::Colon) {
                return Err(SyntaxError::new("expected ':' after 'else'"));
            }
            let _orelse = self.parse_suite()?;
        }
        Ok(Stmt::ActiveOther)
    }

    fn parse_with(&mut self) -> Result<Stmt, SyntaxError> {
        self.pos += 1; // consume `with`
        let header = self.collect_header_until_colon()?;
        if header.is_empty() {
            return Err(SyntaxError::new("expected context expression in 'with'"));
        }
        let _body = self.parse_suite()?;
        Ok(Stmt::ActiveOther)
    }

    fn is_match_statement(&self) -> bool {
        // Check if `match` is followed on this line by `<expr> : Newline Indent case`
        // without top-level `=` or `+=`.
        let mut idx = self.pos + 1;
        let mut depth = 0usize;
        let mut colon_idx = None;
        while idx < self.tokens.len() {
            match &self.tokens[idx].kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Assign | TokenKind::AugAssign | TokenKind::Semi if depth == 0 => {
                    return false;
                }
                TokenKind::Colon if depth == 0 => {
                    colon_idx = Some(idx);
                    break;
                }
                TokenKind::Newline if depth == 0 => break,
                _ => {}
            }
            idx += 1;
        }
        if let Some(c_idx) = colon_idx {
            if c_idx > self.pos + 1
                && self.tokens.get(c_idx + 1).map(|t| &t.kind) == Some(&TokenKind::Newline)
                && self.tokens.get(c_idx + 2).map(|t| &t.kind) == Some(&TokenKind::Indent)
            {
                return true;
            }
        }
        false
    }

    fn parse_match(&mut self) -> Result<Stmt, SyntaxError> {
        self.pos += 1; // consume `match`
        let subject = self.collect_header_until_colon()?;
        if subject.is_empty() {
            return Err(SyntaxError::new("expected subject expression in 'match'"));
        }
        check_expr(&subject, ExprContext::Top)?;
        if !self.eat(&TokenKind::Newline) || !self.eat(&TokenKind::Indent) {
            return Err(SyntaxError::new("expected indented block after 'match'"));
        }
        let mut has_case = false;
        while !self.eat(&TokenKind::Dedent) {
            if self.eat(&TokenKind::Newline) {
                continue;
            }
            if !self.eat_ident("case") {
                return Err(SyntaxError::new("expected 'case' inside 'match' block"));
            }
            has_case = true;
            let pattern = self.collect_header_until_colon()?;
            if pattern.is_empty() {
                return Err(SyntaxError::new("expected pattern after 'case'"));
            }
            let _case_body = self.parse_suite()?;
        }
        if !has_case {
            return Err(SyntaxError::new(
                "'match' block must contain at least one 'case'",
            ));
        }
        Ok(Stmt::ActiveOther)
    }

    fn parse_suite(&mut self) -> Result<Vec<Stmt>, SyntaxError> {
        if self.eat(&TokenKind::Newline) {
            if !self.eat(&TokenKind::Indent) {
                return Err(SyntaxError::new("expected an indented block"));
            }
            let mut stmts = Vec::new();
            while !self.eat(&TokenKind::Dedent) {
                if self.is_eof() {
                    return Err(SyntaxError::new("unexpected EOF while parsing suite"));
                }
                if self.eat(&TokenKind::Newline) {
                    continue;
                }
                self.parse_stmt(&mut stmts)?;
            }
            if stmts.is_empty() {
                return Err(SyntaxError::new("empty suite"));
            }
            Ok(stmts)
        } else {
            let mut stmts = Vec::new();
            self.parse_simple_stmt_line(&mut stmts)?;
            if stmts.is_empty() {
                return Err(SyntaxError::new("expected statement after ':'"));
            }
            Ok(stmts)
        }
    }

    fn consume_balanced_group(&mut self) -> Result<Vec<Token>, SyntaxError> {
        let mut depth = 0usize;
        let mut inner = Vec::new();
        while self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            match tok.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    if depth > 0 {
                        inner.push(tok);
                    }
                    depth += 1;
                }
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(inner);
                    }
                    inner.push(tok);
                }
                _ => {
                    inner.push(tok);
                }
            }
        }
        Err(SyntaxError::new("unclosed bracket group"))
    }

    fn collect_header_until_colon(&mut self) -> Result<Vec<Token>, SyntaxError> {
        // Collect tokens on the current logical line up to the header `:` at bracket depth 0
        // (accounting for any `lambda ...:` colons at depth 0).
        let mut depth = 0usize;
        let mut pending_lambdas = 0usize;
        let mut out = Vec::new();

        while self.pos < self.tokens.len() {
            let kind = &self.tokens[self.pos].kind;
            match kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    depth += 1;
                    out.push(self.tokens[self.pos].clone());
                    self.pos += 1;
                }
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                    out.push(self.tokens[self.pos].clone());
                    self.pos += 1;
                }
                TokenKind::Ident(s) if s == "lambda" && depth == 0 => {
                    pending_lambdas += 1;
                    out.push(self.tokens[self.pos].clone());
                    self.pos += 1;
                }
                TokenKind::Colon if depth == 0 => {
                    if pending_lambdas > 0 {
                        pending_lambdas -= 1;
                        out.push(self.tokens[self.pos].clone());
                        self.pos += 1;
                    } else {
                        self.pos += 1; // consume header `:`
                        return Ok(out);
                    }
                }
                TokenKind::Newline if depth == 0 => {
                    return Err(SyntaxError::new("expected ':'"));
                }
                _ => {
                    out.push(self.tokens[self.pos].clone());
                    self.pos += 1;
                }
            }
        }
        Err(SyntaxError::new("expected ':'"))
    }

    fn collect_until_newline(&mut self) -> Result<Vec<Token>, SyntaxError> {
        let mut out = Vec::new();
        while self.pos < self.tokens.len() {
            if self.eat(&TokenKind::Newline) {
                return Ok(out);
            }
            out.push(self.tokens[self.pos].clone());
            self.pos += 1;
        }
        Ok(out)
    }

    fn parse_simple_stmt_line(&mut self, out: &mut Vec<Stmt>) -> Result<(), SyntaxError> {
        let line_tokens = self.collect_until_newline()?;
        if line_tokens.is_empty() {
            return Ok(());
        }

        // Split by `;` at bracket depth 0.
        let mut parts: Vec<&[Token]> = Vec::new();
        let mut start = 0;
        let mut depth = 0usize;
        for (i, tok) in line_tokens.iter().enumerate() {
            match tok.kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                }
                TokenKind::Semi if depth == 0 => {
                    let slice = &line_tokens[start..i];
                    if slice.is_empty() {
                        return Err(SyntaxError::new("invalid syntax: unexpected ';'"));
                    }
                    parts.push(slice);
                    start = i + 1;
                }
                _ => {}
            }
        }
        let tail = &line_tokens[start..];
        if !tail.is_empty() {
            parts.push(tail);
        }

        for part in parts {
            let stmt = parse_single_simple_stmt(part)?;
            out.push(stmt);
        }
        Ok(())
    }
}

fn find_top_level_ident(tokens: &[Token], target: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, tok) in tokens.iter().enumerate() {
        match &tok.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
            }
            TokenKind::Ident(s) if depth == 0 && s == target => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_single_simple_stmt(tokens: &[Token]) -> Result<Stmt, SyntaxError> {
    if tokens.is_empty() {
        return Err(SyntaxError::new("empty statement"));
    }

    if let TokenKind::Ident(first) = &tokens[0].kind {
        match first.as_str() {
            "pass" => {
                if tokens.len() != 1 {
                    return Err(SyntaxError::new("unexpected tokens after 'pass'"));
                }
                return Ok(Stmt::InertSimple);
            }
            "break" | "continue" => {
                if tokens.len() != 1 {
                    return Err(SyntaxError::new(format!(
                        "unexpected tokens after '{}'",
                        first
                    )));
                }
                return Ok(Stmt::ActiveOther);
            }
            "import" => {
                validate_import_stmt(&tokens[1..])?;
                return Ok(Stmt::InertSimple);
            }
            "from" => {
                validate_from_import_stmt(&tokens[1..])?;
                return Ok(Stmt::InertSimple);
            }
            "global" => {
                validate_name_list(&tokens[1..])?;
                return Ok(Stmt::InertSimple);
            }
            "nonlocal" => {
                validate_name_list(&tokens[1..])?;
                return Ok(Stmt::ActiveOther);
            }
            "assert" => {
                let rest = &tokens[1..];
                if rest.is_empty() {
                    return Err(SyntaxError::new("expected expression after 'assert'"));
                }
                if let Some(comma_idx) = find_top_level_token(rest, &TokenKind::Comma) {
                    let test_expr = &rest[..comma_idx];
                    let msg_expr = &rest[comma_idx + 1..];
                    if test_expr.is_empty() || msg_expr.is_empty() {
                        return Err(SyntaxError::new("invalid 'assert' syntax"));
                    }
                    let test_runs = check_expr(test_expr, ExprContext::Top)?;
                    let msg_runs = check_expr(msg_expr, ExprContext::Top)?;
                    return Ok(Stmt::Assert {
                        runs_code: test_runs || msg_runs,
                    });
                } else {
                    let runs_code = check_expr(rest, ExprContext::Top)?;
                    return Ok(Stmt::Assert { runs_code });
                }
            }
            "return" => {
                if tokens.len() > 1 {
                    check_expr(&tokens[1..], ExprContext::TupleAllowed)?;
                }
                return Ok(Stmt::ActiveOther);
            }
            "raise" => {
                if tokens.len() > 1 {
                    let rest = &tokens[1..];
                    if let Some(from_idx) = find_top_level_ident(rest, "from") {
                        let exc = &rest[..from_idx];
                        let cause = &rest[from_idx + 1..];
                        if exc.is_empty() || cause.is_empty() {
                            return Err(SyntaxError::new("invalid 'raise ... from' syntax"));
                        }
                        check_expr(exc, ExprContext::Top)?;
                        check_expr(cause, ExprContext::Top)?;
                    } else {
                        check_expr(rest, ExprContext::Top)?;
                    }
                }
                return Ok(Stmt::ActiveOther);
            }
            "del" => {
                let rest = &tokens[1..];
                if rest.is_empty() {
                    return Err(SyntaxError::new("expected target after 'del'"));
                }
                validate_assign_target(rest, true)?;
                return Ok(Stmt::ActiveOther);
            }
            "yield" => {
                let rest = &tokens[1..];
                if let Some(Token {
                    kind: TokenKind::Ident(s),
                }) = rest.first()
                {
                    if s == "from" {
                        if rest.len() < 2 {
                            return Err(SyntaxError::new("expected expression after 'yield from'"));
                        }
                        check_expr(&rest[1..], ExprContext::Top)?;
                        return Ok(Stmt::Expr { runs_code: true });
                    }
                }
                if !rest.is_empty() {
                    check_expr(rest, ExprContext::TupleAllowed)?;
                }
                return Ok(Stmt::Expr { runs_code: true });
            }
            "type" => {
                // Check PEP 695 `type Alias = ...` or `type Alias[T] = ...`.
                if let Some(rhs) = try_parse_pep695_type_alias(tokens)? {
                    check_expr(rhs, ExprContext::Top)?;
                    return Ok(Stmt::InertSimple);
                }
            }
            kw if is_hard_keyword(kw)
                && !matches!(kw, "True" | "False" | "None" | "not" | "lambda" | "await") =>
            {
                return Err(SyntaxError::new(format!(
                    "unexpected keyword '{}' in simple statement",
                    kw
                )));
            }
            _ => {}
        }
    }

    // Scan at bracket depth 0 for `AugAssign`, `Colon` (`AnnAssign`), or `Assign`.
    let mut depth = 0usize;
    let mut pending_lambdas = 0usize;
    let mut aug_assign_pos = None;
    let mut colon_pos = None;
    let mut assign_positions = Vec::new();

    for (i, tok) in tokens.iter().enumerate() {
        match &tok.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
            }
            TokenKind::Ident(s) if depth == 0 && s == "lambda" => {
                pending_lambdas += 1;
            }
            TokenKind::Colon if depth == 0 => {
                if pending_lambdas > 0 {
                    pending_lambdas -= 1;
                } else if colon_pos.is_none()
                    && assign_positions.is_empty()
                    && aug_assign_pos.is_none()
                {
                    colon_pos = Some(i);
                } else {
                    return Err(SyntaxError::new("unexpected ':' in statement"));
                }
            }
            TokenKind::AugAssign if depth == 0 => {
                if aug_assign_pos.is_some() || colon_pos.is_some() || !assign_positions.is_empty() {
                    return Err(SyntaxError::new("invalid augmented assignment"));
                }
                aug_assign_pos = Some(i);
            }
            TokenKind::Assign if depth == 0 => {
                if pending_lambdas == 0 {
                    if aug_assign_pos.is_some() {
                        return Err(SyntaxError::new(
                            "invalid assignment after augmented assignment",
                        ));
                    }
                    assign_positions.push(i);
                }
            }
            TokenKind::Walrus if depth == 0 => {
                return Err(SyntaxError::new(
                    "cannot use assignment expressions at top level without parentheses",
                ));
            }
            _ => {}
        }
    }

    // 1. `AugAssign` (`target += value`)
    if let Some(aug_idx) = aug_assign_pos {
        let target = &tokens[..aug_idx];
        let value = &tokens[aug_idx + 1..];
        if target.is_empty() || value.is_empty() {
            return Err(SyntaxError::new("invalid augmented assignment"));
        }
        validate_assign_target(target, false)?;
        let value_runs_code = check_expr(value, ExprContext::TupleAllowed)?;
        return Ok(Stmt::Assign { value_runs_code });
    }

    // 2. `AnnAssign` (`target: annotation [= value]`)
    if let Some(c_idx) = colon_pos {
        let target = &tokens[..c_idx];
        if target.is_empty() {
            return Err(SyntaxError::new(
                "expected target before ':' in annotated assignment",
            ));
        }
        validate_assign_target(target, false)?;
        if assign_positions.len() > 1 {
            return Err(SyntaxError::new(
                "cannot chain multiple '=' in annotated assignment",
            ));
        }
        if let Some(&eq_idx) = assign_positions.first() {
            let annotation = &tokens[c_idx + 1..eq_idx];
            let value = &tokens[eq_idx + 1..];
            if annotation.is_empty() || value.is_empty() {
                return Err(SyntaxError::new("invalid annotated assignment"));
            }
            // Validate annotation syntax, but only check `value` for running code!
            check_expr(annotation, ExprContext::Top)?;
            let value_runs_code = check_expr(value, ExprContext::TupleAllowed)?;
            return Ok(Stmt::Assign { value_runs_code });
        } else {
            let annotation = &tokens[c_idx + 1..];
            if annotation.is_empty() {
                return Err(SyntaxError::new("expected type annotation after ':'"));
            }
            check_expr(annotation, ExprContext::Top)?;
            return Ok(Stmt::Assign {
                value_runs_code: false,
            });
        }
    }

    // 3. `Assign` (`t1 = t2 = ... = value`)
    if !assign_positions.is_empty() {
        let mut start = 0;
        for &eq_idx in &assign_positions {
            let target = &tokens[start..eq_idx];
            if target.is_empty() {
                return Err(SyntaxError::new("missing assignment target before '='"));
            }
            validate_assign_target(target, true)?;
            start = eq_idx + 1;
        }
        let value = &tokens[start..];
        if value.is_empty() {
            return Err(SyntaxError::new("expected expression after '='"));
        }
        let value_runs_code = check_expr(value, ExprContext::TupleAllowed)?;
        return Ok(Stmt::Assign { value_runs_code });
    }

    // 4. Bare expression statement (`Expr`)
    let runs_code = check_expr(tokens, ExprContext::TupleAllowed)?;
    Ok(Stmt::Expr { runs_code })
}

fn try_parse_pep695_type_alias(tokens: &[Token]) -> Result<Option<&[Token]>, SyntaxError> {
    if tokens.len() < 4 {
        return Ok(None);
    }
    match &tokens[1].kind {
        TokenKind::Ident(name) if !is_hard_keyword(name) => {}
        _ => return Ok(None),
    }

    let mut idx = 2;
    if tokens.get(idx).map(|t| &t.kind) == Some(&TokenKind::LBracket) {
        let mut depth = 0usize;
        while idx < tokens.len() {
            match tokens[idx].kind {
                TokenKind::LBracket | TokenKind::LParen | TokenKind::LBrace => depth += 1,
                TokenKind::RBracket | TokenKind::RParen | TokenKind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        idx += 1;
                        break;
                    }
                }
                _ => {}
            }
            idx += 1;
        }
    }

    if tokens.get(idx).map(|t| &t.kind) == Some(&TokenKind::Assign) {
        let rhs = &tokens[idx + 1..];
        if rhs.is_empty() {
            return Err(SyntaxError::new(
                "expected type expression after '=' in type alias",
            ));
        }
        Ok(Some(rhs))
    } else {
        Ok(None)
    }
}

fn find_top_level_token(tokens: &[Token], target: &TokenKind) -> Option<usize> {
    let mut depth = 0usize;
    for (i, tok) in tokens.iter().enumerate() {
        match &tok.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
            }
            k if depth == 0 && k == target => return Some(i),
            _ => {}
        }
    }
    None
}

fn validate_import_stmt(tokens: &[Token]) -> Result<(), SyntaxError> {
    if tokens.is_empty() {
        return Err(SyntaxError::new("expected module name after 'import'"));
    }
    // Each comma-separated item is `ident (. ident)* [as ident]`.
    for item in split_top_level_commas(tokens, false)? {
        if item.is_empty() {
            return Err(SyntaxError::new("invalid 'import' syntax"));
        }
        let module_part = if item.len() >= 2
            && matches!(&item[item.len() - 2].kind, TokenKind::Ident(s) if s == "as")
        {
            match &item[item.len() - 1].kind {
                TokenKind::Ident(alias) if !is_hard_keyword(alias) => &item[..item.len() - 2],
                _ => return Err(SyntaxError::new("invalid 'as' alias in 'import'")),
            }
        } else {
            item
        };
        validate_dotted_name(module_part)?;
    }
    Ok(())
}

fn validate_from_import_stmt(tokens: &[Token]) -> Result<(), SyntaxError> {
    let imp_idx = find_top_level_ident(tokens, "import")
        .ok_or_else(|| SyntaxError::new("expected 'import' in 'from ... import'"))?;
    let from_part = &tokens[..imp_idx];
    let names_part = &tokens[imp_idx + 1..];
    if from_part.is_empty() || names_part.is_empty() {
        return Err(SyntaxError::new("invalid 'from ... import' syntax"));
    }

    // `from_part` starts with zero or more `.` or `...` followed by optional dotted name.
    let mut dot_end = 0;
    while dot_end < from_part.len()
        && matches!(
            from_part[dot_end].kind,
            TokenKind::Dot | TokenKind::Ellipsis
        )
    {
        dot_end += 1;
    }
    if dot_end < from_part.len() {
        validate_dotted_name(&from_part[dot_end..])?;
    }

    if names_part.len() == 1 && names_part[0].kind == TokenKind::Op("*") {
        return Ok(());
    }

    let names_slice = if names_part.first().map(|t| &t.kind) == Some(&TokenKind::LParen)
        && names_part.last().map(|t| &t.kind) == Some(&TokenKind::RParen)
    {
        &names_part[1..names_part.len() - 1]
    } else {
        names_part
    };

    if names_slice.is_empty() {
        return Err(SyntaxError::new("empty import list"));
    }

    for item in split_top_level_commas(names_slice, true)? {
        if item.is_empty() {
            continue;
        }
        match item.len() {
            1 => match &item[0].kind {
                TokenKind::Ident(name) if !is_hard_keyword(name) => {}
                _ => return Err(SyntaxError::new("invalid imported name")),
            },
            3 => match (&item[0].kind, &item[1].kind, &item[2].kind) {
                (TokenKind::Ident(n), TokenKind::Ident(as_kw), TokenKind::Ident(alias))
                    if !is_hard_keyword(n) && as_kw == "as" && !is_hard_keyword(alias) => {}
                _ => return Err(SyntaxError::new("invalid 'import ... as' clause")),
            },
            _ => return Err(SyntaxError::new("invalid item in 'from ... import'")),
        }
    }

    Ok(())
}

fn validate_dotted_name(tokens: &[Token]) -> Result<(), SyntaxError> {
    if tokens.is_empty() || tokens.len() % 2 == 0 {
        return Err(SyntaxError::new("invalid module name"));
    }
    for (i, tok) in tokens.iter().enumerate() {
        if i % 2 == 0 {
            match &tok.kind {
                TokenKind::Ident(name) if !is_hard_keyword(name) => {}
                _ => return Err(SyntaxError::new("invalid identifier in module name")),
            }
        } else if tok.kind != TokenKind::Dot {
            return Err(SyntaxError::new("expected '.' in module name"));
        }
    }
    Ok(())
}

fn validate_name_list(tokens: &[Token]) -> Result<(), SyntaxError> {
    if tokens.is_empty() {
        return Err(SyntaxError::new("expected identifier list"));
    }
    for item in split_top_level_commas(tokens, false)? {
        if item.len() != 1 {
            return Err(SyntaxError::new("expected single identifier"));
        }
        match &item[0].kind {
            TokenKind::Ident(name) if !is_hard_keyword(name) => {}
            _ => return Err(SyntaxError::new("expected valid identifier")),
        }
    }
    Ok(())
}

fn split_top_level_commas(
    tokens: &[Token],
    allow_trailing: bool,
) -> Result<Vec<&[Token]>, SyntaxError> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (i, tok) in tokens.iter().enumerate() {
        match tok.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
            }
            TokenKind::Comma if depth == 0 => {
                let slice = &tokens[start..i];
                if slice.is_empty() {
                    return Err(SyntaxError::new("unexpected ','"));
                }
                parts.push(slice);
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = &tokens[start..];
    if tail.is_empty() {
        if !allow_trailing && !tokens.is_empty() {
            return Err(SyntaxError::new("trailing comma not allowed here"));
        }
    } else {
        parts.push(tail);
    }
    Ok(parts)
}

fn validate_assign_target(tokens: &[Token], allow_tuple: bool) -> Result<(), SyntaxError> {
    if tokens.is_empty() {
        return Err(SyntaxError::new("empty assignment target"));
    }

    if allow_tuple && find_top_level_token(tokens, &TokenKind::Comma).is_some() {
        for elem in split_top_level_commas(tokens, true)? {
            if !elem.is_empty() {
                let elem = if elem.first().map(|t| &t.kind) == Some(&TokenKind::Op("*")) {
                    &elem[1..]
                } else {
                    elem
                };
                validate_assign_target(elem, true)?;
            }
        }
        return Ok(());
    }

    // Strip outer parens or brackets if they wrap the entire target: `(a, b) = x` or `[a, b] = x`.
    if matches!(
        tokens.first().map(|t| &t.kind),
        Some(TokenKind::LParen | TokenKind::LBracket)
    ) && group_end_index(tokens, 0) == Some(tokens.len() - 1)
    {
        if !allow_tuple
            && tokens.len() > 2
            && find_top_level_token(&tokens[1..tokens.len() - 1], &TokenKind::Comma).is_some()
        {
            return Err(SyntaxError::new(
                "tuple/list target not allowed in augmented or annotated assignment",
            ));
        }
        let inner = &tokens[1..tokens.len() - 1];
        if inner.is_empty() {
            return if allow_tuple {
                Ok(())
            } else {
                Err(SyntaxError::new("empty target not allowed"))
            };
        }
        return validate_assign_target(inner, allow_tuple);
    }

    // Must be a valid expression that ends with an identifier, `.attr`, or `[subscript]`
    // and contains no top-level operators!
    check_expr(tokens, ExprContext::Top)?;

    // Ensure no top-level operators exist and the final trailer is assignable.
    let mut i = 0;
    let first = &tokens[0].kind;
    match first {
        TokenKind::Ident(name) if !is_hard_keyword(name) => {
            i += 1;
        }
        TokenKind::LParen
        | TokenKind::LBracket
        | TokenKind::LBrace
        | TokenKind::StringLit { .. }
        | TokenKind::Number => {
            if tokens.len() == 1 {
                return Err(SyntaxError::new("cannot assign to literal"));
            }
            if matches!(
                first,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace
            ) {
                i = group_end_index(tokens, 0).unwrap() + 1;
            } else {
                i += 1;
            }
        }
        _ => return Err(SyntaxError::new("invalid assignment target")),
    }

    let mut last_was_assignable = i == 1 && matches!(first, TokenKind::Ident(_));
    while i < tokens.len() {
        match &tokens[i].kind {
            TokenKind::Dot => {
                if i + 1 >= tokens.len() {
                    return Err(SyntaxError::new("expected attribute name after '.'"));
                }
                match &tokens[i + 1].kind {
                    TokenKind::Ident(attr) if !is_hard_keyword(attr) => {
                        i += 2;
                        last_was_assignable = true;
                    }
                    _ => return Err(SyntaxError::new("invalid attribute name")),
                }
            }
            TokenKind::LBracket => {
                let end =
                    group_end_index(tokens, i).ok_or_else(|| SyntaxError::new("unclosed '['"))?;
                i = end + 1;
                last_was_assignable = true;
            }
            TokenKind::LParen => {
                let end =
                    group_end_index(tokens, i).ok_or_else(|| SyntaxError::new("unclosed '('"))?;
                i = end + 1;
                last_was_assignable = false; // function call is not assignable unless followed by `.attr` or `[sub]`
            }
            _ => {
                return Err(SyntaxError::new("cannot assign to operator or expression"));
            }
        }
    }

    if !last_was_assignable {
        return Err(SyntaxError::new(
            "cannot assign to function call or literal",
        ));
    }

    Ok(())
}

fn group_end_index(tokens: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, tok) in tokens.iter().enumerate().skip(start) {
        match tok.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprContext {
    Top,
    TupleAllowed,
    CallArgs,
    Subscript,
    Brace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprState {
    ExpectOperand,
    AfterOperand,
}

/// Validates expression syntax and returns `Ok(true)` if evaluating `tokens`
/// invokes any `Call`, `Await`, `Yield`, or `YieldFrom`.
fn check_expr(tokens: &[Token], ctx: ExprContext) -> Result<bool, SyntaxError> {
    if tokens.is_empty() {
        return match ctx {
            ExprContext::CallArgs | ExprContext::TupleAllowed | ExprContext::Brace => Ok(false),
            ExprContext::Top | ExprContext::Subscript => {
                Err(SyntaxError::new("expected expression"))
            }
        };
    }

    let mut state = ExprState::ExpectOperand;
    let mut runs_code = false;
    let mut i = 0;

    while i < tokens.len() {
        let kind = &tokens[i].kind;
        match state {
            ExprState::ExpectOperand => match kind {
                // Unary prefix operators: `+`, `-`, `~`, `*`, `**`.
                TokenKind::Op("+" | "-" | "~" | "*" | "**") => {
                    i += 1;
                }
                TokenKind::Ident(kw) if kw == "not" => {
                    i += 1;
                }
                TokenKind::Ident(kw) if kw == "await" => {
                    runs_code = true;
                    i += 1;
                }
                TokenKind::Ident(kw) if kw == "yield" => {
                    runs_code = true;
                    i += 1;
                    if i < tokens.len() {
                        if let TokenKind::Ident(next_kw) = &tokens[i].kind {
                            if next_kw == "from" {
                                i += 1;
                                if i >= tokens.len() {
                                    return Err(SyntaxError::new(
                                        "expected expression after 'yield from'",
                                    ));
                                }
                                continue;
                            }
                        }
                    }
                    // Bare `yield` at end of group or before `,` is a complete operand.
                    if i >= tokens.len() || matches!(tokens[i].kind, TokenKind::Comma) {
                        state = ExprState::AfterOperand;
                    }
                }
                TokenKind::Ident(kw) if kw == "lambda" => {
                    i += 1;
                    // Scan lambda parameter list up to `:` at depth 0.
                    let param_start = i;
                    let mut depth = 0usize;
                    let mut colon_idx = None;
                    while i < tokens.len() {
                        match &tokens[i].kind {
                            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                                depth += 1;
                            }
                            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                                depth = depth.saturating_sub(1);
                            }
                            TokenKind::Colon if depth == 0 => {
                                colon_idx = Some(i);
                                break;
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let c_idx = colon_idx
                        .ok_or_else(|| SyntaxError::new("expected ':' in lambda expression"))?;
                    let params = &tokens[param_start..c_idx];
                    if check_lambda_params(params)? {
                        runs_code = true;
                    }
                    i = c_idx + 1;
                    if i >= tokens.len() {
                        return Err(SyntaxError::new("expected body after ':' in lambda"));
                    }
                }
                TokenKind::Ident(name) => {
                    if !is_atom_ident(name) {
                        return Err(SyntaxError::new(format!(
                            "unexpected keyword '{}' in expression",
                            name
                        )));
                    }
                    state = ExprState::AfterOperand;
                    i += 1;
                }
                TokenKind::Number | TokenKind::Ellipsis => {
                    state = ExprState::AfterOperand;
                    i += 1;
                }
                TokenKind::StringLit { has_active_expr } => {
                    if *has_active_expr {
                        runs_code = true;
                    }
                    state = ExprState::AfterOperand;
                    i += 1;
                }
                TokenKind::LParen => {
                    let end = group_end_index(tokens, i)
                        .ok_or_else(|| SyntaxError::new("unclosed '('"))?;
                    let inner = &tokens[i + 1..end];
                    if !inner.is_empty() && check_expr(inner, ExprContext::TupleAllowed)? {
                        runs_code = true;
                    }
                    state = ExprState::AfterOperand;
                    i = end + 1;
                }
                TokenKind::LBracket => {
                    let end = group_end_index(tokens, i)
                        .ok_or_else(|| SyntaxError::new("unclosed '['"))?;
                    let inner = &tokens[i + 1..end];
                    if !inner.is_empty() && check_expr(inner, ExprContext::TupleAllowed)? {
                        runs_code = true;
                    }
                    state = ExprState::AfterOperand;
                    i = end + 1;
                }
                TokenKind::LBrace => {
                    let end = group_end_index(tokens, i)
                        .ok_or_else(|| SyntaxError::new("unclosed '{'"))?;
                    let inner = &tokens[i + 1..end];
                    if !inner.is_empty() && check_expr(inner, ExprContext::Brace)? {
                        runs_code = true;
                    }
                    state = ExprState::AfterOperand;
                    i = end + 1;
                }
                TokenKind::Colon if ctx == ExprContext::Subscript => {
                    // Slice with omitted start, e.g. `a[:2]` or `a[:]`.
                    i += 1;
                    if i >= tokens.len() || matches!(tokens[i].kind, TokenKind::Comma) {
                        state = ExprState::AfterOperand;
                    }
                }
                _ => {
                    return Err(SyntaxError::new("invalid expression syntax"));
                }
            },
            ExprState::AfterOperand => match kind {
                // Attribute access `.attr`.
                TokenKind::Dot => {
                    if i + 1 >= tokens.len() {
                        return Err(SyntaxError::new("expected attribute name after '.'"));
                    }
                    match &tokens[i + 1].kind {
                        TokenKind::Ident(attr) if !is_hard_keyword(attr) => {
                            i += 2;
                        }
                        _ => return Err(SyntaxError::new("invalid attribute name after '.'")),
                    }
                }
                // Function / method / callable call `primary(...)`!
                TokenKind::LParen => {
                    runs_code = true;
                    let end = group_end_index(tokens, i)
                        .ok_or_else(|| SyntaxError::new("unclosed '(' in call"))?;
                    let inner = &tokens[i + 1..end];
                    if !inner.is_empty() {
                        check_expr(inner, ExprContext::CallArgs)?;
                    }
                    i = end + 1;
                }
                // Subscript `primary[...]`.
                TokenKind::LBracket => {
                    let end = group_end_index(tokens, i)
                        .ok_or_else(|| SyntaxError::new("unclosed '[' in subscript"))?;
                    let inner = &tokens[i + 1..end];
                    if inner.is_empty() {
                        return Err(SyntaxError::new("empty subscript is not allowed"));
                    }
                    if check_expr(inner, ExprContext::Subscript)? {
                        runs_code = true;
                    }
                    i = end + 1;
                }
                // Binary / comparison / walrus operators.
                TokenKind::Op(_) | TokenKind::Walrus => {
                    state = ExprState::ExpectOperand;
                    i += 1;
                }
                TokenKind::Assign if ctx == ExprContext::CallArgs => {
                    // Keyword argument `k=v` inside a call.
                    state = ExprState::ExpectOperand;
                    i += 1;
                }
                TokenKind::Colon if matches!(ctx, ExprContext::Brace | ExprContext::Subscript) => {
                    i += 1;
                    if ctx == ExprContext::Subscript
                        && (i >= tokens.len() || matches!(tokens[i].kind, TokenKind::Comma))
                    {
                        state = ExprState::AfterOperand;
                    } else {
                        state = ExprState::ExpectOperand;
                    }
                }
                TokenKind::Comma => {
                    i += 1;
                    if i < tokens.len() {
                        state = ExprState::ExpectOperand;
                    }
                }
                TokenKind::Ident(kw) => match kw.as_str() {
                    "and" | "or" | "in" | "if" | "else" | "for" => {
                        state = ExprState::ExpectOperand;
                        i += 1;
                    }
                    "async" => {
                        if i + 1 < tokens.len()
                            && matches!(&tokens[i + 1].kind, TokenKind::Ident(s) if s == "for")
                        {
                            state = ExprState::ExpectOperand;
                            i += 2;
                        } else {
                            return Err(SyntaxError::new("expected 'for' after 'async'"));
                        }
                    }
                    "not" => {
                        if i + 1 < tokens.len()
                            && matches!(&tokens[i + 1].kind, TokenKind::Ident(s) if s == "in")
                        {
                            state = ExprState::ExpectOperand;
                            i += 2;
                        } else {
                            return Err(SyntaxError::new("expected 'in' after 'not'"));
                        }
                    }
                    "is" => {
                        if i + 1 < tokens.len()
                            && matches!(&tokens[i + 1].kind, TokenKind::Ident(s) if s == "not")
                        {
                            state = ExprState::ExpectOperand;
                            i += 2;
                        } else {
                            state = ExprState::ExpectOperand;
                            i += 1;
                        }
                    }
                    _ => {
                        return Err(SyntaxError::new(format!(
                            "unexpected identifier '{}' after expression operand",
                            kw
                        )));
                    }
                },
                _ => {
                    return Err(SyntaxError::new(
                        "unexpected token after expression operand",
                    ));
                }
            },
        }
    }

    if state != ExprState::AfterOperand {
        return Err(SyntaxError::new("incomplete expression"));
    }

    Ok(runs_code)
}

fn check_lambda_params(params: &[Token]) -> Result<bool, SyntaxError> {
    if params.is_empty() {
        return Ok(false);
    }
    let mut runs_code = false;
    for item in split_top_level_commas(params, true)? {
        if item.is_empty() {
            continue;
        }
        if let Some(eq_idx) = find_top_level_token(item, &TokenKind::Assign) {
            let default_expr = &item[eq_idx + 1..];
            if default_expr.is_empty() {
                return Err(SyntaxError::new(
                    "missing default value in lambda parameter",
                ));
            }
            if check_expr(default_expr, ExprContext::Top)? {
                runs_code = true;
            }
        }
    }
    Ok(runs_code)
}
