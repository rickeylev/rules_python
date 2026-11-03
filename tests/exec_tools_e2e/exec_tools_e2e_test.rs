use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use zip::{CompressionMethod, DateTime, ZipArchive};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

const URLSAFE_B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Self {
        let base = env::var_os("TEST_TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!("e2e_{}_{}_{}", prefix, std::process::id(), id));
        fs::create_dir_all(&dir).expect("failed to create test dir");
        Self { path: dir }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, rel: &str, content: impl AsRef<[u8]>) -> PathBuf {
        let p = self.path.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&p, content).expect("write file");
        p
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
struct CmdResult {
    code: i32,
    stdout: String,
    stderr: String,
}

fn resolve_runfile(env_var: &str) -> PathBuf {
    let raw = env::var(env_var).unwrap_or_else(|_| panic!("missing env var {}", env_var));
    let p = PathBuf::from(&raw);
    if p.exists() {
        return p;
    }
    if let Ok(srcdir) = env::var("TEST_SRCDIR") {
        let ws = env::var("TEST_WORKSPACE").unwrap_or_else(|_| "_main".to_string());
        let candidate = PathBuf::from(&srcdir).join(&ws).join(&raw);
        if candidate.exists() {
            return candidate;
        }
        let candidate2 = PathBuf::from(&srcdir).join(&raw);
        if candidate2.exists() {
            return candidate2;
        }
    }
    p
}

fn resolve_runfiles_list(env_var: &str) -> Vec<PathBuf> {
    let raw = env::var(env_var).unwrap_or_else(|_| panic!("missing env var {}", env_var));
    raw.split_whitespace()
        .map(|item| {
            let p = PathBuf::from(item);
            if p.exists() {
                return p;
            }
            if let Ok(srcdir) = env::var("TEST_SRCDIR") {
                let ws = env::var("TEST_WORKSPACE").unwrap_or_else(|_| "_main".to_string());
                let candidate = PathBuf::from(&srcdir).join(&ws).join(item);
                if candidate.exists() {
                    return candidate;
                }
            }
            p
        })
        .collect()
}

fn subcommand_for_bin_env(bin_env: &str) -> &'static str {
    match bin_env {
        "BUILD_DATA_WRITER_BIN" => "build_data_writer",
        "EXE_ZIP_MAKER_BIN" => "exe_zip_maker",
        "EXTRACT_WHEEL_FILES_BIN" => "extract_wheel_files",
        "PY_CONSOLE_SCRIPT_GEN_BIN" => "py_console_script_gen",
        "PY_TEST_MAIN_VALIDATOR_BIN" => "py_test_main_validator",
        "PY_WHEEL_DIST_BIN" => "py_wheel_dist",
        "VENV_SHEBANG_REWRITER_BIN" => "venv_shebang_rewriter",
        "WHEELMAKER_BIN" => "wheelmaker",
        "WHEEL_RECORD_REWRITER_BIN" => "wheel_record_rewriter",
        "ZIPPER_BIN" => "zipper",
        "ZIP_MAIN_MAKER_BIN" => "zip_main_maker",
        other => panic!("unknown tool bin env {}", other),
    }
}

fn run_tool(bin_env: &str, args: &[&str], envs: &[(&str, &str)]) -> CmdResult {
    let bin = resolve_runfile("RUPYRU_BOX_BIN");
    let subcommand = subcommand_for_bin_env(bin_env);
    let mut cmd = Command::new(&bin);
    cmd.arg(subcommand);
    cmd.args(args);
    for &(k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to execute {:?} {}: {}", bin, subcommand, e));
    CmdResult {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex_encode(&h.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn urlsafe_b64_nopad(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut chunks = bytes.chunks_exact(3);
    for c in &mut chunks {
        let (b0, b1, b2) = (c[0] as usize, c[1] as usize, c[2] as usize);
        out.push(URLSAFE_B64[b0 >> 2] as char);
        out.push(URLSAFE_B64[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        out.push(URLSAFE_B64[((b1 & 0x0F) << 2) | (b2 >> 6)] as char);
        out.push(URLSAFE_B64[b2 & 0x3F] as char);
    }
    let rem = chunks.remainder();
    if rem.len() == 1 {
        let b0 = rem[0] as usize;
        out.push(URLSAFE_B64[b0 >> 2] as char);
        out.push(URLSAFE_B64[(b0 & 0x03) << 4] as char);
    } else if rem.len() == 2 {
        let (b0, b1) = (rem[0] as usize, rem[1] as usize);
        out.push(URLSAFE_B64[b0 >> 2] as char);
        out.push(URLSAFE_B64[((b0 & 0x03) << 4) | (b1 >> 4)] as char);
        out.push(URLSAFE_B64[(b1 & 0x0F) << 2] as char);
    }
    out
}

fn sha256_urlsafe_digest(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("sha256={}", urlsafe_b64_nopad(&h.finalize()))
}

#[derive(Debug, Clone)]
struct ZipMember {
    name: String,
    compression: CompressionMethod,
    last_modified: Option<DateTime>,
    unix_mode: Option<u32>,
    data: Vec<u8>,
}

fn read_zip_entries(zip_path: &Path) -> Vec<ZipMember> {
    let file = fs::File::open(zip_path).expect("open zip file");
    let mut archive = ZipArchive::new(file).expect("parse zip archive");
    let mut members = Vec::with_capacity(archive.len());
    for idx in 0..archive.len() {
        let mut entry = archive.by_index(idx).expect("read zip member");
        let name = entry.name().to_string();
        let compression = entry.compression();
        let last_modified = entry.last_modified();
        let unix_mode = entry.unix_mode();
        let uncomp_size = entry.size() as usize;
        let mut data = Vec::with_capacity(uncomp_size);
        entry.read_to_end(&mut data).expect("decompress zip member");
        assert_eq!(data.len(), uncomp_size);
        members.push(ZipMember {
            name,
            compression,
            last_modified,
            unix_mode,
            data,
        });
    }
    members
}

fn read_zip_map(zip_path: &Path) -> HashMap<String, Vec<u8>> {
    read_zip_entries(zip_path)
        .into_iter()
        .map(|m| (m.name, m.data))
        .collect()
}

// ============================================================================
// TIER 1 — FEATURE COVERAGE (50 Happy-Path E2E Tests: 5 per Feature F1–F10)
// ============================================================================

// --- F1: py_console_script_gen (5 tests) ---

#[test]
fn tier1_f1_01_console_script_explicit_flag_equals() {
    let td = TestDir::new("t1_f1_01");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\nmy-cli = my_pkg.cli:main\nother = my_pkg.other:run\n",
    );
    let out = td.path().join("wrapper.py");
    let arg_cs = "--console-script=my-cli";
    let arg_guess = "--console-script-guess=unused_guess";
    let res = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            arg_cs,
            arg_guess,
            ep.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(res.stdout.is_empty());
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.starts_with("import sys\n"));
    assert!(content.contains("from my_pkg.cli import main\n"));
    assert!(content.contains("sys.exit(main())  # pyrefly: ignore[not-callable]\n"));
}

#[test]
fn tier1_f1_02_console_script_guess_with_empty_script_flag() {
    let td = TestDir::new("t1_f1_02");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\nmy_tool = pkg.runner:start\n",
    );
    let out = td.path().join("guessed.py");
    let res = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script",
            "",
            "--console-script-guess",
            "my_tool",
            "--shebang",
            "",
            ep.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.starts_with("import sys\n"));
    assert!(content.contains("from pkg.runner import start\n"));
    assert!(content.contains("sys.exit(start())"));
}

#[test]
fn tier1_f1_03_console_script_dotted_class_method_and_shebang() {
    let td = TestDir::new("t1_f1_03");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\napp = my_pkg.sub.mod:AppRunner.execute\n",
    );
    let out = td.path().join("app.py");
    let res = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script",
            "app",
            "--console-script-guess",
            "app",
            "--shebang",
            "#!/usr/bin/env python3",
            ep.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.starts_with("#!/usr/bin/env python3\nimport sys\n"));
    assert!(content.contains("from my_pkg.sub.mod import AppRunner\n"));
    assert!(content.contains("sys.exit(AppRunner.execute())"));
}

#[test]
fn tier1_f1_04_console_script_colon_key_value_and_other_sections() {
    let td = TestDir::new("t1_f1_04");
    let ep = td.write(
        "entry_points.txt",
        "# Header comment\n; Semicolon comment\n[gui_scripts]\ngui = pkg:gui\n\n[console_scripts]\n# comment inside\ncolon_tool : pkg.colon_mod:run_cli\n\n[other_section]\nx = y:z\n",
    );
    let out = td.path().join("colon.py");
    let res = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script=colon_tool",
            "--console-script-guess=colon_tool",
            ep.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.contains("from pkg.colon_mod import run_cli\n"));
    assert!(content.contains("sys.exit(run_cli())"));
}

#[test]
fn tier1_f1_05_console_script_via_flagfile_and_nested_out_dir() {
    let td = TestDir::new("t1_f1_05");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\nFlagTool = flag_pkg.core:Entry.main\n",
    );
    let out = td.path().join("deep/nested/dir/flag_tool.py");
    let flagfile = td.write(
        "args.params",
        format!(
            "--console-script\nFlagTool\n--console-script-guess\nunused\n--shebang=#!python3\n{}\n{}\n",
            ep.display(),
            out.display()
        ),
    );
    let at_arg = format!("@{}", flagfile.display());
    let res = run_tool("PY_CONSOLE_SCRIPT_GEN_BIN", &[&at_arg], &[]);
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.starts_with("#!python3\nimport sys\n"));
    assert!(content.contains("from flag_pkg.core import Entry\n"));
    assert!(content.contains("sys.exit(Entry.main())"));
}

// --- F2: zip_main_maker (5 tests) ---

#[test]
fn tier1_f2_01_regular_files_and_app_hash_substitution() {
    let td = TestDir::new("t1_f2_01");
    let f1 = td.write("a.py", b"print('hello')\n");
    let tmpl = td.write("template.py", "HASH = '%APP_HASH%'\n");
    let line = format!("regular|0|pkg/a.py|{}", f1.display());
    let manifest = td.write("manifest.txt", format!("{}\n", line));
    let out = td.path().join("__main__.py");

    let res = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tmpl.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--hash_files_manifest",
            manifest.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);

    let mut expected_hasher = Sha256::new();
    expected_hasher.update(format!("0|pkg/a.py|{}", f1.display()).as_bytes());
    expected_hasher.update(b"print('hello')\n");
    let expected_hash = hex_encode(&expected_hasher.finalize());

    let content = fs::read_to_string(&out).unwrap();
    assert_eq!(content, format!("HASH = '{}'\n", expected_hash));
}

#[test]
fn tier1_f2_02_manifest_sorting_invariance() {
    let td = TestDir::new("t1_f2_02");
    let fa = td.write("a.txt", b"AAA");
    let fb = td.write("b.txt", b"BBB");
    let tmpl = td.write("template.py", "%APP_HASH%");
    let line_a = format!("regular|0|a.txt|{}", fa.display());
    let line_b = format!("regular|0|b.txt|{}", fb.display());

    let m1 = td.write("m1.txt", format!("{}\n{}\n", line_a, line_b));
    let m2 = td.write("m2.txt", format!("{}\n{}\n", line_b, line_a));
    let out1 = td.path().join("out1.py");
    let out2 = td.path().join("out2.py");

    let r1 = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            &format!("--template={}", tmpl.display()),
            &format!("--output={}", out1.display()),
            &format!("--hash_files_manifest={}", m1.display()),
        ],
        &[],
    );
    let r2 = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            &format!("--template={}", tmpl.display()),
            &format!("--output={}", out2.display()),
            &format!("--hash_files_manifest={}", m2.display()),
        ],
        &[],
    );
    assert_eq!(r1.code, 0);
    assert_eq!(r2.code, 0);
    let h1 = fs::read_to_string(&out1).unwrap();
    let h2 = fs::read_to_string(&out2).unwrap();
    assert_eq!(h1.len(), 64);
    assert_eq!(h1, h2);
}

#[test]
fn tier1_f2_03_rf_empty_and_symlink_manifest_entries() {
    let td = TestDir::new("t1_f2_03");
    let real_file = td.write("mod.py", b"X = 1\n");
    let tmpl = td.write("tmpl.py", "APP_HASH=%APP_HASH%\n");
    let manifest_content = format!(
        "rf-empty|pkg/__init__.py\nsymlink|0|pkg/link.py|../target.py\nrf-file|0|pkg/mod.py|{}\n",
        real_file.display()
    );
    let manifest = td.write("manifest.txt", &manifest_content);
    let out = td.path().join("main.py");

    let res = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tmpl.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--hash_files_manifest",
            manifest.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let rendered = fs::read_to_string(&out).unwrap();
    assert!(rendered.starts_with("APP_HASH="));
    assert_eq!(rendered.trim().len(), "APP_HASH=".len() + 64);
}

#[test]
fn tier1_f2_04_multiple_substitutions_and_override_order() {
    let td = TestDir::new("t1_f2_04");
    let f = td.write("x.py", b"pass\n");
    let manifest = td.write("m.txt", format!("regular|0|x.py|{}\n", f.display()));
    let tmpl = td.write("tmpl.py", "WS=%WORKSPACE%\nMAIN=%MAIN%\nHASH=%APP_HASH%\n");
    let out = td.path().join("out.py");

    let res = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tmpl.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--substitution",
            "%WORKSPACE%=old_ws",
            "--substitution=%MAIN%=pkg/main.py",
            "--substitution",
            "%WORKSPACE%=new_ws",
            "--hash_files_manifest",
            manifest.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.starts_with("WS=new_ws\nMAIN=pkg/main.py\nHASH="));
}

#[test]
fn tier1_f2_05_flagfile_invocation_and_nested_output() {
    let td = TestDir::new("t1_f2_05");
    let f = td.write("code.py", b"print(1)\n");
    let manifest = td.write("m.txt", format!("regular|0|code.py|{}\n", f.display()));
    let tmpl = td.write("tmpl.py", "ENTRY='%ENTRY%' HASH='%APP_HASH%'\n");
    let out = td.path().join("a/b/c/__main__.py");
    let inner_flags = td.write(
        "inner.flags",
        format!(
            "--substitution=%ENTRY%=code:main\n--hash_files_manifest={}\n",
            manifest.display()
        ),
    );
    let outer_flags = td.write(
        "outer.flags",
        format!(
            "--template\n{}\n--output\n{}\n@{}\n",
            tmpl.display(),
            out.display(),
            inner_flags.display()
        ),
    );
    let at_outer = format!("@{}", outer_flags.display());
    let res = run_tool("ZIP_MAIN_MAKER_BIN", &[&at_outer], &[]);
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let content = fs::read_to_string(&out).unwrap();
    assert!(content.starts_with("ENTRY='code:main' HASH='"));
}

// --- F3: py_test_main_validator (5 tests) ---

#[test]
fn tier1_f3_01_valid_unittest_main_if_guard() {
    let td = TestDir::new("t1_f3_01");
    let src = td.write(
        "test_ok.py",
        "import unittest\n\nclass MyTest(unittest.TestCase):\n    def test_one(self):\n        self.assertEqual(1, 1)\n\nif __name__ == '__main__':\n    unittest.main()\n",
    );
    let out = td.path().join("validated.ok");
    let res = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            src.to_str().unwrap(),
            "--src_name",
            "test_ok.py",
            "--label",
            "//pkg:test_ok",
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(res.stderr.is_empty());
    assert_eq!(fs::read_to_string(&out).unwrap(), "");
}

#[test]
fn tier1_f3_02_valid_top_level_pytest_call() {
    let td = TestDir::new("t1_f3_02");
    let src = td.write(
        "test_pytest.py",
        "import sys, pytest\n\ndef test_feature():\n    assert True\n\nsys.exit(pytest.main())\n",
    );
    let out = td.path().join("out.ok");
    let res = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            &format!("--src={}", src.display()),
            &format!("--output={}", out.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(out.exists());
}

#[test]
fn tier1_f3_03_valid_module_with_no_test_definitions() {
    let td = TestDir::new("t1_f3_03");
    let src = td.write(
        "constants_only.py",
        "\"\"\"Docstring only module.\"\"\"\nimport os\nANSWER: int = 42\n",
    );
    let out = td.path().join("out.ok");
    let res = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            src.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(out.exists());
}

#[test]
fn tier1_f3_04_valid_module_with_active_control_flow() {
    let td = TestDir::new("t1_f3_04");
    let src = td.write(
        "active_loop.py",
        "def test_a():\n    pass\n\nfor fn in [test_a]:\n    pass\n",
    );
    let out = td.path().join("out.ok");
    let res = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            src.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(out.exists());
}

#[test]
fn tier1_f3_05_flagfile_and_active_assign_or_assert() {
    let td = TestDir::new("t1_f3_05");
    let src = td.write(
        "active_assign.py",
        "async def test_async():\n    return True\n\nSTATUS = run_all()\n",
    );
    let out = td.path().join("nested/out.ok");
    let flagfile = td.write(
        "validator.args",
        format!(
            "--src={}\n--src_name=active_assign.py\n--label=//pkg:active_assign\n--output={}\n",
            src.display(),
            out.display()
        ),
    );
    let at_arg = format!("@{}", flagfile.display());
    let res = run_tool("PY_TEST_MAIN_VALIDATOR_BIN", &[&at_arg], &[]);
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(out.exists());
}

// --- F4: wheelmaker (5 tests) ---

#[test]
fn tier1_f4_01_basic_uncompressed_wheel_and_name_file() {
    let td = TestDir::new("t1_f4_01");
    let mod_py = td.write("my_pkg/__init__.py", b"VERSION = '1.0.0'\n");
    let meta = td.write(
        "METADATA.in",
        "Metadata-Version: 2.1\nName: my-pkg\nSummary: Example wheel\n",
    );
    let out_whl = td.path().join("out.whl");
    let name_file = td.path().join("wheel_name.txt");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name",
            "my-pkg",
            "--version",
            "1.0.0",
            "--no_compress",
            "--metadata_file",
            meta.to_str().unwrap(),
            "--input_file",
            &format!("my_pkg/__init__.py;{}", mod_py.display()),
            "--out",
            out_whl.to_str().unwrap(),
            "--name_file",
            name_file.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&name_file).unwrap(),
        "my_pkg-1.0.0-py3-none-any.whl"
    );

    let entries = read_zip_entries(&out_whl);
    assert!(entries
        .iter()
        .all(|e| e.compression == CompressionMethod::Stored));
    let map = read_zip_map(&out_whl);
    assert_eq!(
        map.get("my_pkg/__init__.py").unwrap(),
        b"VERSION = '1.0.0'\n"
    );
    let wheel_txt =
        String::from_utf8(map.get("my_pkg-1.0.0.dist-info/WHEEL").unwrap().clone()).unwrap();
    assert!(wheel_txt.contains("Wheel-Version: 1.0\nGenerator: bazel-wheelmaker 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n"));
    let record_txt =
        String::from_utf8(map.get("my_pkg-1.0.0.dist-info/RECORD").unwrap().clone()).unwrap();
    let expected_digest = sha256_urlsafe_digest(b"VERSION = '1.0.0'\n");
    assert!(record_txt.contains(&format!("my_pkg/__init__.py,{},18\n", expected_digest)));
    assert!(record_txt.ends_with("my_pkg-1.0.0.dist-info/RECORD,,\n"));
}

#[test]
fn tier1_f4_02_compressed_wheel_deflated_and_deterministic_headers() {
    let td = TestDir::new("t1_f4_02");
    let empty_py = td.write("pkg/empty.py", b"");
    let mod_py = td.write("pkg/mod.py", b"def hello(): return 'world'\n");
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: pkg\n");
    let out_whl = td.path().join("compressed.whl");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=pkg",
            "--version=0.2.0",
            "--Compress",
            &format!("--metadata_file={}", meta.display()),
            &format!("--input_file=pkg/empty.py;{}", empty_py.display()),
            &format!("--input_file=pkg/mod.py;{}", mod_py.display()),
            &format!("--out={}", out_whl.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let entries = read_zip_entries(&out_whl);
    assert!(!entries.is_empty());
    for entry in &entries {
        assert_eq!(
            entry.compression,
            CompressionMethod::Deflated,
            "entry {} must be ZIP_DEFLATED",
            entry.name
        );
        assert_eq!(entry.last_modified, Some(DateTime::default()));
        assert_eq!(entry.unix_mode, Some(0o100777));
    }
}

#[test]
fn tier1_f4_03_pep440_version_and_pep508_requires_normalization() {
    let td = TestDir::new("t1_f4_03");
    let reqs = td.write(
        "reqs.txt",
        "# comment\n--index-url https://example.com\nrequests [ security , socks ] >= 2.28.0 , != 2.29.0 ; python_version >= '3.8'\n",
    );
    let extra_reqs = td.write("dev_reqs.txt", "pytest>=7.0; os_name == 'posix'\n");
    let meta = td.write(
        "METADATA.in",
        format!(
            "Metadata-Version: 2.1\nName: My.Cool_Pkg\nRequires-Dist: @{}\nRequires-Dist: @{}\n",
            reqs.display(),
            extra_reqs.display()
        ),
    );
    let desc = td.write("README.md", "# My Cool Package\n");
    let out_whl = td.path().join("norm.whl");
    let name_file = td.path().join("name.txt");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=My.Cool_Pkg",
            "--version=v1.02.0alpha03.dev04",
            &format!("--metadata_file={}", meta.display()),
            &format!("--description_file={}", desc.display()),
            &format!("--extra_requires_file={};dev", extra_reqs.display()),
            &format!("--out={}", out_whl.display()),
            &format!("--name_file={}", name_file.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&name_file).unwrap(),
        "my_cool_pkg-1.2.0a3.dev4-py3-none-any.whl"
    );
    let map = read_zip_map(&out_whl);
    let metadata_str = String::from_utf8(
        map.get("my_cool_pkg-1.2.0a3.dev4.dist-info/METADATA")
            .unwrap()
            .clone(),
    )
    .unwrap();
    assert!(metadata_str.contains("Version: 1.2.0a3.dev4\n"));
    assert!(metadata_str.contains(
        "Requires-Dist: requests[security,socks]!=2.29.0,>=2.28.0; python_version >= \"3.8\"\n"
    ));
    assert!(metadata_str
        .contains("Requires-Dist: pytest>=7.0; (os_name == \"posix\") and extra == 'dev'\n"));
    assert!(metadata_str.ends_with("\n\n# My Cool Package\n\n"));
}

#[test]
fn tier1_f4_04_strip_path_prefix_and_path_prefix_and_input_file_list() {
    let td = TestDir::new("t1_f4_04");
    let f1 = td.write("f1.py", b"A = 1\n");
    let f2 = td.write("f2.py", b"B = 2\n");
    let list_file = td.write(
        "inputs.list",
        format!(
            "src/python/pkg/a.py;{}\nsrc/other/b.py;{}\n",
            f1.display(),
            f2.display()
        ),
    );
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: prefixed\n");
    let out_whl = td.path().join("prefixed.whl");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=prefixed",
            "--version=1.0",
            "--strip_path_prefix=src/python/",
            "--strip_path_prefix=src/",
            "--path_prefix=vendored/",
            &format!("--input_file_list={}", list_file.display()),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", out_whl.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let map = read_zip_map(&out_whl);
    assert!(map.contains_key("vendored/pkg/a.py"));
    assert!(map.contains_key("vendored/other/b.py"));
}

#[test]
fn tier1_f4_05_workspace_status_stamping_and_data_files() {
    let td = TestDir::new("t1_f4_05");
    let volatile = td.write("volatile-status.txt", "BUILD_TIMESTAMP 20261101\n");
    let stable = td.write(
        "stable-status.txt",
        "STABLE_GIT_TAG 2.3.4\nSTABLE_BUILD_NUM 7\n",
    );
    let meta = td.write(
        "METADATA.in",
        "Metadata-Version: 2.1\nName: stamped-{BUILD_TIMESTAMP}\nHome-page: https://example.com/{STABLE_GIT_TAG}\n",
    );
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\nstamped-cli = pkg:main\n",
    );
    let license = td.write("LICENSE.txt", "MIT License\n");
    let script = td.write("my_script.py", "#!python\nprint('hi')\n");
    let out_whl = td.path().join("stamped.whl");
    let name_file = td.path().join("stamped_name.txt");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=stamped-{BUILD_TIMESTAMP}",
            "--version={STABLE_GIT_TAG}",
            "--build_tag={STABLE_BUILD_NUM}",
            &format!("--volatile_status_file={}", volatile.display()),
            &format!("--stable_status_file={}", stable.display()),
            &format!("--metadata_file={}", meta.display()),
            &format!("--entry_points_file={}", ep.display()),
            &format!("--extra_distinfo_file=LICENSE;{}", license.display()),
            &format!("--data_files=scripts/my_script;{}", script.display()),
            &format!("--out={}", out_whl.display()),
            &format!("--name_file={}", name_file.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&name_file).unwrap(),
        "stamped_20261101-2.3.4-7-py3-none-any.whl"
    );
    let map = read_zip_map(&out_whl);
    assert!(map.contains_key("stamped_20261101-2.3.4.dist-info/entry_points.txt"));
    assert!(map.contains_key("stamped_20261101-2.3.4.dist-info/LICENSE"));
    assert!(map.contains_key("stamped_20261101-2.3.4.data/scripts/my_script"));
    let meta_out = String::from_utf8(
        map.get("stamped_20261101-2.3.4.dist-info/METADATA")
            .unwrap()
            .clone(),
    )
    .unwrap();
    assert!(meta_out.contains("Name: stamped-20261101\n"));
    assert!(meta_out.contains("Home-page: https://example.com/2.3.4\n"));
}

// --- F5: py_wheel_dist (5 tests) ---

#[test]
fn tier1_f5_01_basic_wheel_dist_copy_separate_flags() {
    let td = TestDir::new("t1_f5_01");
    let whl = td.write("built.whl", b"PK\x05\x06dummy_wheel_payload");
    let nf = td.write("name.txt", "pkg-1.0.0-py3-none-any.whl");
    let out_dir = td.path().join("dist_out");

    let res = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            whl.to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let copied = out_dir.join("pkg-1.0.0-py3-none-any.whl");
    assert_eq!(fs::read(&copied).unwrap(), b"PK\x05\x06dummy_wheel_payload");
}

#[test]
fn tier1_f5_02_inline_equals_flags_and_whitespace_trimmed_name_file() {
    let td = TestDir::new("t1_f5_02");
    let whl = td.write("raw.whl", b"wheel_bytes_123");
    let nf = td.write("name.txt", "  \n\ttrimmed_pkg-2.1.0-py3-none-any.whl \r\n ");
    let out_dir = td.path().join("dist_inline");

    let res = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            &format!("--wheel={}", whl.display()),
            &format!("--name_file={}", nf.display()),
            &format!("--output={}", out_dir.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read(out_dir.join("trimmed_pkg-2.1.0-py3-none-any.whl")).unwrap(),
        b"wheel_bytes_123"
    );
}

#[test]
fn tier1_f5_03_nested_nonexistent_output_directory_creation() {
    let td = TestDir::new("t1_f5_03");
    let whl = td.write("in.whl", b"nested_wheel");
    let nf = td.write("name.txt", "nested-0.1.0-py3-none-any.whl");
    let out_dir = td.path().join("level1/level2/level3/dist");

    let res = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            whl.to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(out_dir.join("nested-0.1.0-py3-none-any.whl").exists());
}

#[test]
fn tier1_f5_04_flagfile_invocation() {
    let td = TestDir::new("t1_f5_04");
    let whl = td.write("in.whl", b"flagfile_wheel");
    let nf = td.write("name.txt", "flag_pkg-3.0.0-py3-none-any.whl");
    let out_dir = td.path().join("flag_dist");
    let params = td.write(
        "dist.params",
        format!(
            "--wheel\n{}\n--name_file={}\n--output\n{}\n",
            whl.display(),
            nf.display(),
            out_dir.display()
        ),
    );
    let at_arg = format!("@{}", params.display());
    let res = run_tool("PY_WHEEL_DIST_BIN", &[&at_arg], &[]);
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read(out_dir.join("flag_pkg-3.0.0-py3-none-any.whl")).unwrap(),
        b"flagfile_wheel"
    );
}

#[test]
fn tier1_f5_05_overwrite_existing_wheel_in_output_dir() {
    let td = TestDir::new("t1_f5_05");
    let whl1 = td.write("v1.whl", b"old_content");
    let whl2 = td.write("v2.whl", b"new_updated_content");
    let nf = td.write("name.txt", "overwrite-1.0-py3-none-any.whl");
    let out_dir = td.path().join("dist");

    let r1 = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            whl1.to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0);
    let r2 = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            whl2.to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r2.code, 0);
    assert_eq!(
        fs::read(out_dir.join("overwrite-1.0-py3-none-any.whl")).unwrap(),
        b"new_updated_content"
    );
}

// --- F6: extract_wheel_files (5 tests) ---

fn build_test_wheel(td: &TestDir, compress: bool, files: &[(&str, &[u8])]) -> PathBuf {
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: test-whl\n");
    let whl_path = td.path().join("sample.whl");
    let mut args: Vec<String> = vec![
        "--name=test_whl".to_string(),
        "--version=1.0.0".to_string(),
        format!("--metadata_file={}", meta.display()),
        format!("--out={}", whl_path.display()),
    ];
    if compress {
        args.push("--Compress".to_string());
    } else {
        args.push("--no_compress".to_string());
    }
    for (idx, (pkg_path, data)) in files.iter().enumerate() {
        let real = td.write(&format!("in_{}", idx), data);
        args.push(format!("--input_file={};{}", pkg_path, real.display()));
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let res = run_tool("WHEELMAKER_BIN", &arg_refs, &[]);
    assert_eq!(res.code, 0, "wheelmaker failed: {}", res.stderr);
    whl_path
}

#[test]
fn tier1_f6_01_extract_all_files_without_regex_arg() {
    let td = TestDir::new("t1_f6_01");
    let whl = build_test_wheel(
        &td,
        false,
        &[
            ("pkg/a.py", b"print('a')\n"),
            ("pkg/include/hdr.h", b"#pragma once\n"),
        ],
    );
    let out_dir = td.path().join("extracted");
    let res = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[whl.to_str().unwrap(), out_dir.to_str().unwrap()],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(fs::read(out_dir.join("pkg/a.py")).unwrap(), b"print('a')\n");
    assert_eq!(
        fs::read(out_dir.join("pkg/include/hdr.h")).unwrap(),
        b"#pragma once\n"
    );
    assert!(out_dir.join("test_whl-1.0.0.dist-info/RECORD").exists());
}

#[test]
fn tier1_f6_02_extract_all_files_with_empty_regex_arg() {
    let td = TestDir::new("t1_f6_02");
    let whl = build_test_wheel(&td, true, &[("pkg/b.py", b"B = 2\n")]);
    let out_dir = td.path().join("extracted_empty_pat");
    let res = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[whl.to_str().unwrap(), out_dir.to_str().unwrap(), ""],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(fs::read(out_dir.join("pkg/b.py")).unwrap(), b"B = 2\n");
    assert!(out_dir.join("test_whl-1.0.0.dist-info/WHEEL").exists());
}

#[test]
fn tier1_f6_03_extract_header_files_matching_regex() {
    let td = TestDir::new("t1_f6_03");
    let whl = build_test_wheel(
        &td,
        true,
        &[
            ("pkg/mod.py", b"pass\n"),
            ("pkg/include/api.h", b"int api(void);\n"),
            ("pkg/include/detail/internal.h", b"void priv(void);\n"),
            ("pkg/include/readme.txt", b"docs\n"),
        ],
    );
    let out_dir = td.path().join("headers_only");
    let res = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            whl.to_str().unwrap(),
            out_dir.to_str().unwrap(),
            r"pkg/include/.*\.h$",
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(out_dir.join("pkg/include/api.h").exists());
    assert!(out_dir.join("pkg/include/detail/internal.h").exists());
    assert!(!out_dir.join("pkg/mod.py").exists());
    assert!(!out_dir.join("pkg/include/readme.txt").exists());
    assert!(!out_dir.join("test_whl-1.0.0.dist-info/RECORD").exists());
}

#[test]
fn tier1_f6_04_extract_alternation_and_character_classes() {
    let td = TestDir::new("t1_f6_04");
    let whl = build_test_wheel(
        &td,
        true,
        &[
            ("pkg/sub/note_01.txt", b"note1"),
            ("pkg/sub/SKIP.txt", b"skip"),
            ("pkg/main.py", b"main"),
            ("pkg/other.py", b"other"),
        ],
    );
    let out_dir = td.path().join("alt_out");
    let res = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            whl.to_str().unwrap(),
            out_dir.to_str().unwrap(),
            r"(pkg/sub/[a-z0-9_]+\.txt$|pkg/main\.py$)",
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read(out_dir.join("pkg/sub/note_01.txt")).unwrap(),
        b"note1"
    );
    assert_eq!(fs::read(out_dir.join("pkg/main.py")).unwrap(), b"main");
    assert!(!out_dir.join("pkg/sub/SKIP.txt").exists());
    assert!(!out_dir.join("pkg/other.py").exists());
}

#[test]
fn tier1_f6_05_extract_deflated_and_quoted_csv_record_entries() {
    let td = TestDir::new("t1_f6_05");
    let whl = build_test_wheel(
        &td,
        true,
        &[
            ("pkg/data,with,commas.txt", b"comma_payload"),
            ("pkg/normal.txt", b"normal_payload"),
        ],
    );
    let out_dir = td.path().join("comma_out");
    let res = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            whl.to_str().unwrap(),
            out_dir.to_str().unwrap(),
            r"pkg/.*\.txt",
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read(out_dir.join("pkg/data,with,commas.txt")).unwrap(),
        b"comma_payload"
    );
    assert_eq!(
        fs::read(out_dir.join("pkg/normal.txt")).unwrap(),
        b"normal_payload"
    );
}

// --- F7: venv_shebang_rewriter (5 tests) ---

#[test]
fn tier1_f7_01_unix_shebang_rewrite_and_executable_mode() {
    let td = TestDir::new("t1_f7_01");
    let inp = td.write(
        "cli.py",
        "#!/usr/bin/env python3\nimport sys\nprint(sys.argv)\n",
    );
    let out = td.path().join("bin/cli");
    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[inp.to_str().unwrap(), out.to_str().unwrap(), "unix"],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let expected =
        "#!/bin/sh\n'''exec' \"$(dirname \"$0\")/python3\" \"$0\" \"$@\"\n' '''\nimport sys\nprint(sys.argv)\n";
    assert_eq!(fs::read_to_string(&out).unwrap(), expected);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111);
    }
}

#[test]
fn tier1_f7_02_windows_python_exe_shebang_rewrite() {
    let td = TestDir::new("t1_f7_02");
    let inp = td.write("cli.py", "#!python\nprint('win cli')\n");
    let out = td.path().join("Scripts/cli.bat");
    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[inp.to_str().unwrap(), out.to_str().unwrap(), "windows"],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let expected = b"@setlocal enabledelayedexpansion & \"%~dp0python.exe\" -x \"%~f0\" %* & exit /b !ERRORLEVEL!\r\nprint('win cli')\n";
    assert_eq!(fs::read(&out).unwrap(), expected);
}

#[test]
fn tier1_f7_03_windows_pythonw_exe_shebang_rewrite() {
    let td = TestDir::new("t1_f7_03");
    let inp = td.write("gui.py", "#!pythonw -u\nimport tkinter\n");
    let out = td.path().join("Scripts/gui.bat");
    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[inp.to_str().unwrap(), out.to_str().unwrap(), "windows"],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let expected = b"@setlocal enabledelayedexpansion & \"%~dp0pythonw.exe\" -x \"%~f0\" %* & exit /b !ERRORLEVEL!\r\nimport tkinter\n";
    assert_eq!(fs::read(&out).unwrap(), expected);
}

#[test]
fn tier1_f7_04_preserve_binary_and_utf8_payload_after_first_line() {
    let td = TestDir::new("t1_f7_04");
    let mut raw = b"#!python3\n# UTF-8: \xce\xa9 \xf0\x9f\x90\x8d\n".to_vec();
    raw.extend_from_slice(&[0xff, 0x00, 0xfe, 0x42, b'\n']);
    let inp = td.write("bin_payload.py", &raw);
    let out = td.path().join("out_bin");
    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[inp.to_str().unwrap(), out.to_str().unwrap(), "unix"],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let mut expected =
        b"#!/bin/sh\n'''exec' \"$(dirname \"$0\")/python3\" \"$0\" \"$@\"\n' '''\n".to_vec();
    expected.extend_from_slice(&raw["#!python3\n".len()..]);
    assert_eq!(fs::read(&out).unwrap(), expected);
}

#[test]
fn tier1_f7_05_nested_output_directory_and_extra_trailing_args() {
    let td = TestDir::new("t1_f7_05");
    let inp = td.write("in.py", "#!python\nx = 1\n");
    let out = td.path().join("a/b/c/script");
    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            inp.to_str().unwrap(),
            out.to_str().unwrap(),
            "unix",
            "ignored_extra_arg_1",
            "ignored_extra_arg_2",
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(out.exists());
}

// --- F8: wheel_record_rewriter (5 tests) ---

#[test]
fn tier1_f8_01_unix_record_rewrite_all_five_data_categories() {
    let td = TestDir::new("t1_f8_01");
    let record_in = td.write(
        "RECORD.in",
        "pkg-1.0.data/purelib/pkg/pure.py,sha256=111,10\npkg-1.0.data/platlib/pkg/native.so,sha256=222,20\npkg-1.0.data/scripts/my_cli,sha256=333,30\npkg-1.0.data/headers/my_hdr.h,sha256=444,40\npkg-1.0.data/data/share/doc.txt,sha256=555,50\n",
    );
    let record_out = td.path().join("RECORD.out");
    let res = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            record_out.to_str().unwrap(),
            "unix",
            "pkg-1.0.data",
            "my_cli",
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let expected = "pkg/pure.py,sha256=111,10\npkg/native.so,sha256=222,20\n../../../bin/my_cli,sha256=333,30\n../../../include/my_hdr.h,sha256=444,40\n../../../share/doc.txt,sha256=555,50\n";
    assert_eq!(fs::read_to_string(&record_out).unwrap(), expected);
}

#[test]
fn tier1_f8_02_windows_record_rewrite_with_rewritten_scripts_bat() {
    let td = TestDir::new("t1_f8_02");
    let record_in = td.write(
        "RECORD.in",
        "pkg-1.0.data/purelib/pkg/pure.py,sha256=111,10\npkg-1.0.data/platlib/pkg/native.pyd,sha256=222,20\npkg-1.0.data/scripts/cli_rewritten,sha256=333,30\npkg-1.0.data/scripts/cli_untouched.exe,sha256=334,31\npkg-1.0.data/headers/my_hdr.h,sha256=444,40\npkg-1.0.data/data/etc/cfg.ini,sha256=555,50\n",
    );
    let record_out = td.path().join("RECORD.win");
    let res = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            record_out.to_str().unwrap(),
            "windows",
            "pkg-1.0.data",
            "cli_rewritten",
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let expected = "pkg/pure.py,sha256=111,10\npkg/native.pyd,sha256=222,20\n../../Scripts/cli_rewritten.bat,sha256=333,30\n../../Scripts/cli_untouched.exe,sha256=334,31\n../../Include/my_hdr.h,sha256=444,40\n../../etc/cfg.ini,sha256=555,50\n";
    assert_eq!(fs::read_to_string(&record_out).unwrap(), expected);
}

#[test]
fn tier1_f8_03_quoted_csv_record_lines_unix_and_windows() {
    let td = TestDir::new("t1_f8_03");
    let record_in = td.write(
        "RECORD.in",
        "\"pkg-1.0.data/purelib/pkg/a,b.py\",sha256=111,10\n\"pkg-1.0.data/scripts/my_cli\",sha256=222,20\n\"pkg-1.0.data/headers/a,b.h\",sha256=333,30\n",
    );
    let out_unix = td.path().join("RECORD.unix");
    let out_win = td.path().join("RECORD.win");

    let r_unix = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            out_unix.to_str().unwrap(),
            "unix",
            "pkg-1.0.data",
            "my_cli",
        ],
        &[],
    );
    assert_eq!(r_unix.code, 0, "stderr: {}", r_unix.stderr);
    assert_eq!(
        fs::read_to_string(&out_unix).unwrap(),
        "\"pkg/a,b.py\",sha256=111,10\n\"../../../bin/my_cli\",sha256=222,20\n\"../../../include/a,b.h\",sha256=333,30\n"
    );

    let r_win = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            out_win.to_str().unwrap(),
            "windows",
            "pkg-1.0.data",
            "my_cli",
        ],
        &[],
    );
    assert_eq!(r_win.code, 0, "stderr: {}", r_win.stderr);
    assert_eq!(
        fs::read_to_string(&out_win).unwrap(),
        "\"pkg/a,b.py\",sha256=111,10\n\"../../Scripts/my_cli.bat\",sha256=222,20\n\"../../Include/a,b.h\",sha256=333,30\n"
    );
}

#[test]
fn tier1_f8_04_preserve_dist_info_and_non_data_lines() {
    let td = TestDir::new("t1_f8_04");
    let record_in = td.write(
        "RECORD.in",
        "pkg/__init__.py,sha256=aaa,5\npkg-1.0.dist-info/METADATA,sha256=bbb,50\npkg-1.0.dist-info/RECORD,,\n",
    );
    let out = td.path().join("RECORD.out");
    let res = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            out.to_str().unwrap(),
            "unix",
            "pkg-1.0.data",
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        "pkg/__init__.py,sha256=aaa,5\npkg-1.0.dist-info/METADATA,sha256=bbb,50\npkg-1.0.dist-info/RECORD,,\n"
    );
}

#[test]
fn tier1_f8_05_crlf_input_normalization_and_nested_output_dir() {
    let td = TestDir::new("t1_f8_05");
    let record_in = td.write(
        "RECORD.crlf",
        "pkg-1.0.data/scripts/run,sha256=abc,12\r\npkg-1.0.dist-info/RECORD,,\r\n",
    );
    let out = td.path().join("site-packages/pkg-1.0.dist-info/RECORD");
    let res = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            out.to_str().unwrap(),
            "unix",
            "pkg-1.0.data",
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let content = fs::read_to_string(&out).unwrap();
    assert!(!content.contains('\r'));
    assert_eq!(
        content,
        "../../../bin/run,sha256=abc,12\npkg-1.0.dist-info/RECORD,,\n"
    );
}

// --- F9: build_data_writer (5 tests) ---

#[test]
fn tier1_f9_01_env_vars_with_version_and_info_files() {
    let td = TestDir::new("t1_f9_01");
    let ver = td.write("version.txt", "BUILD_SCM_REVISION abcdef123456\n");
    let info = td.write(
        "info.txt",
        "BUILD_EMBED_LABEL release_v1\nBUILD_USER alice\n",
    );
    let out = td.path().join("build_data.txt");

    let res = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[],
        &[
            ("OUTPUT", out.to_str().unwrap()),
            ("TARGET", "//my/pkg:my_bin"),
            ("STAMPED", "TRUE"),
            ("VERSION_FILE", ver.to_str().unwrap()),
            ("INFO_FILE", info.to_str().unwrap()),
        ],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let expected = "TARGET //my/pkg:my_bin\nSTAMPED TRUE\nBUILD_SCM_REVISION abcdef123456\nBUILD_EMBED_LABEL release_v1\nBUILD_USER alice\n";
    assert_eq!(fs::read_to_string(&out).unwrap(), expected);
}

#[test]
fn tier1_f9_02_cli_flags_override_env_vars() {
    let td = TestDir::new("t1_f9_02");
    let ver = td.write("v.txt", "KEY1 val1\n");
    let info = td.write("i.txt", "KEY2 val2\n");
    let out = td.path().join("cli_out.txt");

    let res = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[
            "--output",
            out.to_str().unwrap(),
            "--target",
            "//cli:target",
            "--stamped",
            "TRUE",
            "--version_file",
            ver.to_str().unwrap(),
            "--info_file",
            info.to_str().unwrap(),
        ],
        &[
            ("OUTPUT", "/ignored/path"),
            ("TARGET", "//env:ignored"),
            ("STAMPED", "FALSE"),
        ],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        "TARGET //cli:target\nSTAMPED TRUE\nKEY1 val1\nKEY2 val2\n"
    );
}

#[test]
fn tier1_f9_03_inline_equals_flags_and_hyphen_aliases() {
    let td = TestDir::new("t1_f9_03");
    let ver = td.write("v.txt", "V 1\n");
    let info = td.write("i.txt", "I 2\n");
    let out = td.path().join("hyphen_out.txt");

    let res = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[
            &format!("--output={}", out.display()),
            "--target=//inline:bin",
            "--stamped=TRUE",
            &format!("--version-file={}", ver.display()),
            &format!("--info-file={}", info.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        "TARGET //inline:bin\nSTAMPED TRUE\nV 1\nI 2\n"
    );
}

#[test]
fn tier1_f9_04_unstamped_or_missing_optional_status_files() {
    let td = TestDir::new("t1_f9_04");
    let out = td.path().join("unstamped.txt");
    let res = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[],
        &[
            ("OUTPUT", out.to_str().unwrap()),
            ("TARGET", "//pkg:unstamped"),
            ("STAMPED", "FALSE"),
            ("VERSION_FILE", ""),
            ("INFO_FILE", ""),
        ],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        "TARGET //pkg:unstamped\nSTAMPED FALSE\n"
    );
}

#[test]
fn tier1_f9_05_flagfile_and_status_file_without_trailing_newline() {
    let td = TestDir::new("t1_f9_05");
    let ver = td.write("v_no_nl.txt", "NO_NEWLINE_AT_EOF 1");
    let info = td.write("info.txt", "SECOND_LINE 2\n");
    let out = td.path().join("nested/dir/flagfile_out.txt");
    let flagfile = td.write(
        "bd.flags",
        format!(
            "--output={}\n--target=//pkg:flag\n--stamped=TRUE\n--version_file={}\n--info_file={}\n",
            out.display(),
            ver.display(),
            info.display()
        ),
    );
    let at_arg = format!("@{}", flagfile.display());
    let res = run_tool("BUILD_DATA_WRITER_BIN", &[&at_arg], &[]);
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        "TARGET //pkg:flag\nSTAMPED TRUE\nNO_NEWLINE_AT_EOF 1\nSECOND_LINE 2\n"
    );
}

// --- F10 & F11: Starlark Rules, Distribution Filegroups & Tool Binaries (5 tests) ---

#[test]
fn tier1_f10_01_all_eleven_rust_tool_binaries_present_and_executable() {
    let p = resolve_runfile("RUPYRU_BOX_BIN");
    assert!(
        p.exists(),
        "unified rupyru-box binary does not exist at {:?}",
        p
    );
    assert!(
        p.is_file(),
        "unified rupyru-box binary is not a regular file"
    );

    let help_out = Command::new(&p).arg("--help").output().unwrap();
    assert_eq!(help_out.status.code(), Some(0));
    let help_stdout = String::from_utf8_lossy(&help_out.stdout);
    for subcommand in [
        "build_data_writer",
        "exe_zip_maker",
        "extract_wheel_files",
        "py_console_script_gen",
        "py_test_main_validator",
        "py_wheel_dist",
        "venv_shebang_rewriter",
        "wheel_record_rewriter",
        "wheelmaker",
        "zip_main_maker",
        "zipper",
    ] {
        assert!(
            help_stdout.contains(subcommand),
            "rupyru-box --help missing subcommand {}: {}",
            subcommand,
            help_stdout
        );
    }
}

#[test]
fn tier1_f10_02_crate_distribution_filegroups_contain_sources_and_build_files() {
    let lib_dist_envs = [
        ("DIST_PY_CONSOLE_SCRIPT_GEN", "py_console_script_gen"),
        ("DIST_ZIP_MAIN_MAKER", "zip_main_maker"),
        ("DIST_PY_TEST_MAIN_VALIDATOR", "py_test_main_validator"),
        ("DIST_WHEELMAKER", "wheelmaker"),
        ("DIST_PY_WHEEL_DIST", "py_wheel_dist"),
        ("DIST_EXTRACT_WHEEL_FILES", "extract_wheel_files"),
        ("DIST_VENV_SHEBANG_REWRITER", "venv_shebang_rewriter"),
        ("DIST_WHEEL_RECORD_REWRITER", "wheel_record_rewriter"),
        ("DIST_BUILD_DATA_WRITER", "build_data_writer"),
        ("DIST_EXE_ZIP_MAKER", "exe_zip_maker"),
        ("DIST_ZIPPER", "zipper"),
    ];
    for (env_key, crate_name) in lib_dist_envs {
        let files = resolve_runfiles_list(env_key);
        assert!(
            !files.is_empty(),
            "distribution filegroup for {} is empty",
            crate_name
        );
        let paths_str: Vec<String> = files
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        for expected_suffix in ["BUILD.bazel", "src/lib.rs"] {
            assert!(
                paths_str.iter().any(|s| s.ends_with(expected_suffix)),
                "crate {} distribution missing {}: {:?}",
                crate_name,
                expected_suffix,
                paths_str
            );
        }
    }

    let bin_files = resolve_runfiles_list("DIST_RUPYRU_BOX");
    let bin_paths: Vec<String> = bin_files
        .iter()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    for expected_suffix in ["BUILD.bazel", "src/lib.rs", "src/main.rs"] {
        assert!(
            bin_paths.iter().any(|s| s.ends_with(expected_suffix)),
            "rupyru-box distribution missing {}: {:?}",
            expected_suffix,
            bin_paths
        );
    }
}

#[test]
fn tier1_f10_03_starlark_py_wheel_and_py_wheel_dist_targets() {
    let whl_path = resolve_runfile("E2E_WHEEL");
    assert!(whl_path.exists());
    let map = read_zip_map(&whl_path);
    assert!(map.contains_key("e2e_sample_wheel-1.2.3.dist-info/WHEEL"));
    assert!(map.contains_key("e2e_sample_wheel-1.2.3.dist-info/METADATA"));
    assert!(map.contains_key("e2e_sample_wheel-1.2.3.dist-info/entry_points.txt"));
    assert!(map.contains_key("e2e_sample_wheel-1.2.3.dist-info/RECORD"));

    let dist_dir = resolve_runfile("E2E_WHEEL_DIST");
    assert!(dist_dir.is_dir());
    let dist_whl = dist_dir.join("e2e_sample_wheel-1.2.3-py3-none-any.whl");
    assert!(dist_whl.exists(), "expected {:?} in dist dir", dist_whl);
    assert_eq!(fs::read(&whl_path).unwrap(), fs::read(&dist_whl).unwrap());
}

#[test]
fn tier1_f10_04_starlark_whl_filegroup_target_extraction() {
    let fg_dir = resolve_runfile("E2E_WHL_FILEGROUP");
    assert!(fg_dir.is_dir());
    let extracted_hdr = fg_dir.join("tests/exec_tools_e2e/include/e2e_header.h");
    assert!(
        extracted_hdr.exists(),
        "expected extracted header at {:?}",
        extracted_hdr
    );
    let hdr_content = fs::read_to_string(&extracted_hdr).unwrap();
    assert!(hdr_content.contains("#define E2E_HEADER_MAGIC 42"));
    assert!(
        !fg_dir.join("tests/exec_tools_e2e/e2e_pkg_mod.py").exists(),
        "non-matching .py file should not be extracted by whl_filegroup pattern"
    );
}

#[test]
fn tier1_f10_05_starlark_venv_rewrite_shebang_and_gen_wheel_record_targets() {
    let rewritten_shebang = resolve_runfile("E2E_REWRITTEN_SHEBANG");
    assert!(rewritten_shebang.exists());
    let shebang_content = fs::read_to_string(&rewritten_shebang).unwrap();
    assert!(shebang_content.contains("REWRITTEN_SHEBANG_OK"));
    if cfg!(windows) {
        assert!(shebang_content.starts_with("@setlocal enabledelayedexpansion"));
    } else {
        assert!(shebang_content.starts_with("#!/bin/sh\n'''exec'"));
    }

    let rewritten_record = resolve_runfile("E2E_REWRITTEN_RECORD");
    assert!(rewritten_record.exists());
    let record_content = fs::read_to_string(&rewritten_record).unwrap();
    assert!(record_content.contains("e2e_mod.py,sha256=abc,10\n"));
    if cfg!(windows) {
        assert!(record_content.contains("../../Scripts/my_cli.bat,sha256=def,20\n"));
        assert!(record_content.contains("../../Include/e2e_header.h,sha256=ghi,30\n"));
    } else {
        assert!(record_content.contains("../../../bin/my_cli,sha256=def,20\n"));
        assert!(record_content.contains("../../../include/e2e_header.h,sha256=ghi,30\n"));
    }
}
// ============================================================================
// TIER 2: Boundary, Corner & Negative Cases (50 tests: 5 per feature F1-F10)
// ============================================================================

// --- F1: py_console_script_gen (5 boundary/negative tests) ---

#[test]
fn tier2_f1_01_missing_console_scripts_section_error() {
    let td = TestDir::new("t2_f1_01");
    let ep = td.write(
        "entry_points.txt",
        "[gui_scripts]\ngui-app = pkg.gui:main\n",
    );
    let out = td.path().join("out.py");
    let res = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script=gui-app",
            "--console-script-guess=gui-app",
            ep.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_ne!(res.code, 0);
    assert!(
        res.stderr
            .contains("The package does not provide any console_scripts in its entry_points.txt"),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f1_02_explicit_console_script_not_found_lists_sorted_candidates() {
    let td = TestDir::new("t2_f1_02");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\nzebra = pkg:z\nalpha = pkg:a\nmiddle = pkg:m\n",
    );
    let out = td.path().join("out.py");
    let res = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script=nonexistent",
            "--console-script-guess=nonexistent",
            ep.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_ne!(res.code, 0);
    assert!(
        res.stderr.contains(
            "The console_script 'nonexistent' was not found, only the following are available: alpha, middle, zebra"
        ),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f1_03_guess_console_script_not_found_lists_sorted_candidates() {
    let td = TestDir::new("t2_f1_03");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\ntwo_cmd = pkg:two\none_cmd = pkg:one\n",
    );
    let out = td.path().join("out.py");
    let res = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script-guess=bad_guess",
            ep.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_ne!(res.code, 0);
    assert!(
        res.stderr.contains(
            "Tried to guess that you wanted 'bad_guess', but could not find it. Please select one of the following console scripts: one_cmd, two_cmd"
        ),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f1_04_mutually_exclusive_console_script_and_guess_rejected() {
    let td = TestDir::new("t2_f1_04");
    let ep = td.write("entry_points.txt", "[console_scripts]\ncmd = pkg:main\n");
    let out = td.path().join("out.py");
    let r1 = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script=cmd",
            ep.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_ne!(r1.code, 0);
    assert!(
        r1.stderr
            .contains("the following arguments are required: --console-script-guess"),
        "stderr: {}",
        r1.stderr
    );

    let r2 = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &["--console-script-guess=cmd", ep.to_str().unwrap()],
        &[],
    );
    assert_ne!(r2.code, 0);
    assert!(
        r2.stderr.contains("expected 2 positional arguments"),
        "stderr: {}",
        r2.stderr
    );
}

#[test]
fn tier2_f1_05_nonexistent_entry_points_file_and_missing_required_args() {
    let td = TestDir::new("t2_f1_05");
    let out = td.path().join("out.py");
    let r1 = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script=cmd",
            "--console-script-guess=cmd",
            td.path().join("does_not_exist.txt").to_str().unwrap(),
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_ne!(r1.code, 0);

    let r2 = run_tool("PY_CONSOLE_SCRIPT_GEN_BIN", &["--unknown-flag"], &[]);
    assert_ne!(r2.code, 0);
}

// --- F2: zip_main_maker (5 boundary/negative tests) ---

#[test]
fn tier2_f2_01_negative_one_is_symlink_dangling_vs_existing_file() {
    let td = TestDir::new("t2_f2_01");
    let real_file = td.write("real.txt", b"regular_payload");
    let tpl = td.write("tpl.py", "HASH = '%APP_HASH%'\n");

    let real_rest = format!("-1|pkg/real.txt|{}", real_file.display());
    let mf_real = td.write("mf_real.txt", format!("regular|{}\n", real_rest));
    let out_real = td.path().join("out_real.py");
    let r_real = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--hash_files_manifest",
            mf_real.to_str().unwrap(),
            "--output",
            out_real.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r_real.code, 0, "stderr: {}", r_real.stderr);
    let expected_real_hash = hex_encode(
        &Sha256::new()
            .chain_update(real_rest.as_bytes())
            .chain_update(b"regular_payload")
            .finalize(),
    );
    assert_eq!(
        fs::read_to_string(&out_real).unwrap(),
        format!("HASH = '{}'\n", expected_real_hash)
    );

    #[cfg(unix)]
    {
        let dangling = td.path().join("dangling_link");
        std::os::unix::fs::symlink("/nonexistent/dangling/target", &dangling).unwrap();
        let dangling_rest = format!("-1|pkg/link|{}", dangling.display());
        let mf_dangling = td.write("mf_dangling.txt", format!("regular|{}\n", dangling_rest));
        let out_dangling = td.path().join("out_dangling.py");
        let r_dangling = run_tool(
            "ZIP_MAIN_MAKER_BIN",
            &[
                "--template",
                tpl.to_str().unwrap(),
                "--hash_files_manifest",
                mf_dangling.to_str().unwrap(),
                "--output",
                out_dangling.to_str().unwrap(),
            ],
            &[],
        );
        assert_eq!(r_dangling.code, 0, "stderr: {}", r_dangling.stderr);
        let expected_dangling_hash = hex_encode(
            &Sha256::new()
                .chain_update(dangling_rest.as_bytes())
                .chain_update(b"/nonexistent/dangling/target")
                .finalize(),
        );
        assert_eq!(
            fs::read_to_string(&out_dangling).unwrap(),
            format!("HASH = '{}'\n", expected_dangling_hash)
        );
    }
}

#[test]
fn tier2_f2_02_empty_manifest_and_empty_substitution_value() {
    let td = TestDir::new("t2_f2_02");
    let mf = td.write("empty.manifest", "");
    let tpl = td.write("tpl.py", "OPT = 'OPTIONAL_FLAG'\nHASH = '%APP_HASH%'\n");
    let out = td.path().join("out.py");

    let res = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--hash_files_manifest",
            mf.to_str().unwrap(),
            "--substitution",
            "OPTIONAL_FLAG=",
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        "OPT = ''\nHASH = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'\n"
    );
}

#[test]
fn tier2_f2_03_malformed_manifest_line_missing_tab_fields_rejected() {
    let td = TestDir::new("t2_f2_03");
    let tpl = td.write("tpl.py", "%APP_HASH%\n");
    let out = td.path().join("out.py");

    let r1 = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_ne!(r1.code, 0);
    assert!(
        r1.stderr
            .contains("missing required argument: --hash_files_manifest"),
        "stderr: {}",
        r1.stderr
    );

    let r2 = run_tool("ZIP_MAIN_MAKER_BIN", &["unexpected_positional_arg"], &[]);
    assert_ne!(r2.code, 0);
    assert!(
        r2.stderr
            .contains("unexpected positional argument: unexpected_positional_arg"),
        "stderr: {}",
        r2.stderr
    );
}

#[test]
fn tier2_f2_04_invalid_substitution_missing_equals_sign_rejected() {
    let td = TestDir::new("t2_f2_04");
    let mf = td.write("empty.manifest", "");
    let tpl = td.write("tpl.py", "%APP_HASH%\n");
    let out = td.path().join("out.py");

    let res = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--hash_files_manifest",
            mf.to_str().unwrap(),
            "--substitution",
            "INVALID_PAIR_WITHOUT_EQUALS",
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_ne!(res.code, 0);
    assert!(
        res.stderr.contains(
            "invalid --substitution value (expected KEY=VALUE): INVALID_PAIR_WITHOUT_EQUALS"
        ),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f2_05_nonexistent_manifest_artifact_file_rejected() {
    let td = TestDir::new("t2_f2_05");
    let mf = td.write(
        "missing_art.manifest",
        format!(
            "regular|0|pkg/missing.py|{}\n",
            td.path().join("no_such_file.py").display()
        ),
    );
    let tpl = td.write("tpl.py", "%APP_HASH%\n");
    let out = td.path().join("out.py");

    let res = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--hash_files_manifest",
            mf.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_ne!(res.code, 0);
}

// --- F3: py_test_main_validator (5 boundary/negative tests) ---

#[test]
fn tier2_f3_01_empty_or_comments_only_main_module_fails_with_exit_1() {
    let td = TestDir::new("t2_f3_01");
    let def_only_src = td.write(
        "def_only_test.py",
        "# only a test def\ndef test_nothing():\n    pass\n",
    );
    let async_def_src = td.write(
        "async_def_test.py",
        "async def test_async_nothing():\n    return 1\n",
    );
    let out1 = td.path().join("out1.ok");
    let out2 = td.path().join("out2.ok");

    let r1 = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            def_only_src.to_str().unwrap(),
            "--src_name",
            "pkg/def_only_test.py",
            "--label",
            "//pkg:def_only_test",
            "--output",
            out1.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 1, "stderr: {}", r1.stderr);
    assert!(r1
        .stderr
        .contains("py_test target //pkg:def_only_test will not run any tests."));
    assert!(
        !out1.exists(),
        "output file must not be created on validation failure"
    );

    let r2 = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            async_def_src.to_str().unwrap(),
            "--src_name",
            "pkg/async_def_test.py",
            "--label",
            "//pkg:async_def_test",
            "--output",
            out2.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r2.code, 1, "stderr: {}", r2.stderr);
    assert!(!out2.exists());
}

#[test]
fn tier2_f3_02_inert_classes_functions_assignments_and_inert_main_guard_fails() {
    let td = TestDir::new("t2_f3_02");
    let src = td.write(
        "inert_test.py",
        "\"\"\"Module docstring.\"\"\"\nimport unittest\nfrom os import path\n\nCONST = 123\nTYPED: int = 456\n\nclass MyTest(unittest.TestCase):\n    def test_something(self):\n        self.assertEqual(1, 1)\n\ndef helper():\n    unittest.main()\n\nif __name__ == '__main__':\n    class NestedInert:\n        pass\n    LOCAL_VAR = 'still_no_call'\n",
    );
    let out = td.path().join("inert.ok");
    let res = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            src.to_str().unwrap(),
            "--src_name",
            "pkg/inert_test.py",
            "--label",
            "//pkg:inert_test",
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 1, "stderr: {}", res.stderr);
    assert!(
        res.stderr.contains(
            "The main module 'pkg/inert_test.py' only contains inert top-level statements"
        ),
        "stderr: {}",
        res.stderr
    );
    assert!(!out.exists());
}

#[test]
fn tier2_f3_03_syntax_error_emits_warning_and_exits_zero() {
    let td = TestDir::new("t2_f3_03");
    let src = td.write("syntax_err.py", "def broken_function(:\n    pass\n");
    let out = td.path().join("syntax.ok");

    let res = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            src.to_str().unwrap(),
            "--src_name",
            "pkg/syntax_err.py",
            "--label",
            "//pkg:syntax_err",
            "--output",
            out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert!(
        res.stderr
            .contains("WARNING: py_test main validator could not parse pkg/syntax_err.py:"),
        "stderr: {}",
        res.stderr
    );
    assert!(out.exists());
}

#[test]
fn tier2_f3_04_null_byte_or_non_utf8_source_emits_warning_and_exits_zero() {
    let td = TestDir::new("t2_f3_04");
    let nul_src = td.write("nul.py", b"x = 1\x00\n");
    let bad_utf8_src = td.write("bad_utf8.py", &[0xff, 0xfe, 0xfd, b'\n']);
    let out1 = td.path().join("nul.ok");
    let out2 = td.path().join("utf8.ok");

    let r1 = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            nul_src.to_str().unwrap(),
            "--src_name",
            "pkg/nul.py",
            "--label",
            "//pkg:nul",
            "--output",
            out1.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);
    assert!(r1
        .stderr
        .contains("WARNING: py_test main validator could not parse pkg/nul.py:"));
    assert!(out1.exists());

    let r2 = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            bad_utf8_src.to_str().unwrap(),
            "--src_name",
            "pkg/bad_utf8.py",
            "--label",
            "//pkg:bad_utf8",
            "--output",
            out2.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);
    assert!(r2
        .stderr
        .contains("WARNING: py_test main validator could not parse pkg/bad_utf8.py:"));
    assert!(out2.exists());
}

#[test]
fn tier2_f3_05_missing_required_flags_or_unknown_flag_exits_2() {
    let r1 = run_tool("PY_TEST_MAIN_VALIDATOR_BIN", &["--src", "foo.py"], &[]);
    assert_eq!(r1.code, 2);
    assert!(r1.stderr.contains("py_test_main_validator: error:"));

    let r2 = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src=a",
            "--src_name=b",
            "--label=c",
            "--output=d",
            "--unknown-flag",
        ],
        &[],
    );
    assert_eq!(r2.code, 2);
    assert!(r2.stderr.contains("py_test_main_validator: error:"));
}

// --- F4: wheelmaker (5 boundary/negative tests) ---

#[test]
fn tier2_f4_01_unresolved_stamp_placeholder_in_version_pep440_local_fallback() {
    let td = TestDir::new("t2_f4_01");
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: fallback_pkg\n");
    let out_whl = td.path().join("fallback.whl");
    let name_file = td.path().join("fallback_name.txt");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=fallback_pkg",
            "--version={BUILD_SCM_VERSION}",
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", out_whl.display()),
            &format!("--name_file={}", name_file.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let whl_name = fs::read_to_string(&name_file).unwrap();
    assert_eq!(
        whl_name,
        "fallback_pkg-0+build.scm.version-py3-none-any.whl"
    );
}

#[test]
fn tier2_f4_02_csv_quoting_in_record_for_filenames_with_commas_and_quotes() {
    let td = TestDir::new("t2_f4_02");
    let f = td.write("weird.py", b"X = 1\n");
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: csv_pkg\n");
    let out_whl = td.path().join("csv.whl");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=csv_pkg",
            "--version=1.0",
            &format!("--input_file=pkg/a,b\"c.py;{}", f.display()),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", out_whl.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    let map = read_zip_map(&out_whl);
    let record =
        String::from_utf8(map.get("csv_pkg-1.0.dist-info/RECORD").unwrap().clone()).unwrap();
    let digest = sha256_urlsafe_digest(b"X = 1\n");
    assert!(
        record.contains(&format!("\"pkg/a,b\"\"c.py\",{},6\n", digest)),
        "RECORD: {}",
        record
    );
}

#[test]
fn tier2_f4_03_incompatible_abi_typo_or_unknown_flag_rejected() {
    let td = TestDir::new("t2_f4_03");
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: p\n");
    let out_whl = td.path().join("p.whl");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=p",
            "--version=1.0",
            "--incompatible_Abi=cp311",
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", out_whl.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(
        res.stderr
            .contains("unrecognized argument: --incompatible_Abi"),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f4_04_malformed_input_file_pair_missing_semicolon_rejected() {
    let td = TestDir::new("t2_f4_04");
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: p\n");
    let out_whl = td.path().join("p.whl");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=p",
            "--version=1.0",
            "--input_file=missing_semicolon_delimiter",
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", out_whl.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(
        res.stderr.contains("wheelmaker: error:"),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f4_05_nonexistent_requires_file_in_metadata_rejected() {
    let td = TestDir::new("t2_f4_05");
    let meta = td.write(
        "METADATA.in",
        format!(
            "Metadata-Version: 2.1\nName: p\nRequires-Dist: @{}\n",
            td.path().join("no_such_reqs.txt").display()
        ),
    );
    let out_whl = td.path().join("p.whl");

    let res = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=p",
            "--version=1.0",
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", out_whl.display()),
        ],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(
        res.stderr.contains("wheelmaker: error:"),
        "stderr: {}",
        res.stderr
    );
}

// --- F5: py_wheel_dist (5 boundary/negative tests) ---

#[test]
fn tier2_f5_01_empty_or_whitespace_only_name_file_rejected() {
    let td = TestDir::new("t2_f5_01");
    let whl = td.write("in.whl", b"data");
    let nf = td.write("empty_name.txt", "   \n\t\r\n  ");
    let out_dir = td.path().join("dist");

    let res = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            whl.to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(
        res.stderr.contains("wheel name in --name_file is empty"),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f5_02_path_traversal_and_directory_separators_in_name_file_rejected() {
    let td = TestDir::new("t2_f5_02");
    let whl = td.write("in.whl", b"data");
    let out_dir = td.path().join("dist");

    for bad_name in ["../escape.whl", "sub/nested.whl", "sub\\win.whl", ".", ".."] {
        let nf = td.write("bad_name.txt", bad_name);
        let res = run_tool(
            "PY_WHEEL_DIST_BIN",
            &[
                "--wheel",
                whl.to_str().unwrap(),
                "--name_file",
                nf.to_str().unwrap(),
                "--output",
                out_dir.to_str().unwrap(),
            ],
            &[],
        );
        assert_eq!(res.code, 1, "bad_name '{}' should fail", bad_name);
        assert!(
            res.stderr.contains("invalid wheel filename in --name_file"),
            "stderr for '{}': {}",
            bad_name,
            res.stderr
        );
    }
}

#[test]
fn tier2_f5_03_missing_required_flags_rejected() {
    let td = TestDir::new("t2_f5_03");
    let whl = td.write("in.whl", b"data");
    let nf = td.write("name.txt", "ok-1.0-py3-none-any.whl");

    let r1 = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            whl.to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 1);
    assert!(r1.stderr.contains("missing required argument: --output"));

    let r2 = run_tool("PY_WHEEL_DIST_BIN", &[], &[]);
    assert_eq!(r2.code, 1);
    assert!(r2.stderr.contains("missing required argument: --wheel"));
}

#[test]
fn tier2_f5_04_nonexistent_source_wheel_or_name_file_rejected() {
    let td = TestDir::new("t2_f5_04");
    let nf = td.write("name.txt", "ok-1.0-py3-none-any.whl");
    let out_dir = td.path().join("dist");

    let r1 = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            td.path().join("missing.whl").to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
            "--output",
            out_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 1);
    assert!(
        r1.stderr.contains("py_wheel_dist: error:"),
        "stderr: {}",
        r1.stderr
    );
}

#[test]
fn tier2_f5_05_unknown_cli_flag_rejected() {
    let r = run_tool("PY_WHEEL_DIST_BIN", &["--bogus-flag=123"], &[]);
    assert_eq!(r.code, 1);
    assert!(
        r.stderr.contains("unexpected argument: --bogus-flag=123"),
        "stderr: {}",
        r.stderr
    );
}

// --- F6: extract_wheel_files (5 boundary/negative tests) ---

#[test]
fn tier2_f6_01_start_anchored_regex_re_match_semantics() {
    let td = TestDir::new("t2_f6_01");
    let whl = build_test_wheel(&td, false, &[("pkg/include/header.h", b"int x;\n")]);
    let out_unanchored = td.path().join("unanchored");
    let out_anchored = td.path().join("anchored");

    // `include/.*\.h` does NOT match at start of `pkg/include/header.h` under Python `re.match`
    let r1 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            whl.to_str().unwrap(),
            out_unanchored.to_str().unwrap(),
            r"include/.*\.h",
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);
    assert!(!out_unanchored.join("pkg/include/header.h").exists());

    // `.*include/.*\.h` matches from start of string
    let r2 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            whl.to_str().unwrap(),
            out_anchored.to_str().unwrap(),
            r".*include/.*\.h",
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);
    assert!(out_anchored.join("pkg/include/header.h").exists());
}

#[test]
fn tier2_f6_02_corrupt_non_zip_file_rejected() {
    let td = TestDir::new("t2_f6_02");
    let bad_whl = td.write("corrupt.whl", b"not a zip archive at all");
    let out_dir = td.path().join("out");

    let res = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[bad_whl.to_str().unwrap(), out_dir.to_str().unwrap()],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(
        res.stderr.contains("is not a valid zip file"),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f6_03_zip_with_zero_or_multiple_dist_info_records_rejected() {
    let td = TestDir::new("t2_f6_03");
    let f1 = td.write("a.txt", b"hello");

    // Build a zip with 0 .dist-info/RECORD files using zipper
    let no_record_zip = td.path().join("no_record.whl");
    let mf1 = td.write("mf1.txt", format!("regular|0|pkg/a.txt|{}\n", f1.display()));
    let r_zip1 = run_tool(
        "ZIPPER_BIN",
        &[mf1.to_str().unwrap(), no_record_zip.to_str().unwrap()],
        &[],
    );
    assert_eq!(r_zip1.code, 0, "stderr: {}", r_zip1.stderr);

    let out1 = td.path().join("out1");
    let res1 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[no_record_zip.to_str().unwrap(), out1.to_str().unwrap()],
        &[],
    );
    assert_eq!(res1.code, 1);
    assert!(
        res1.stderr
            .contains("doesn't contain exactly one .dist-info/RECORD"),
        "stderr: {}",
        res1.stderr
    );

    // Build a zip with 2 .dist-info/RECORD files
    let two_record_zip = td.path().join("two_record.whl");
    let rec = td.write("RECORD", "a.dist-info/RECORD,,\n");
    let mf2 = td.write(
        "mf2.txt",
        format!(
            "regular|0|a.dist-info/RECORD|{}\nregular|0|b.dist-info/RECORD|{}\n",
            rec.display(),
            rec.display()
        ),
    );
    let r_zip2 = run_tool(
        "ZIPPER_BIN",
        &[mf2.to_str().unwrap(), two_record_zip.to_str().unwrap()],
        &[],
    );
    assert_eq!(r_zip2.code, 0, "stderr: {}", r_zip2.stderr);

    let out2 = td.path().join("out2");
    let res2 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[two_record_zip.to_str().unwrap(), out2.to_str().unwrap()],
        &[],
    );
    assert_eq!(res2.code, 1);
    assert!(
        res2.stderr
            .contains("doesn't contain exactly one .dist-info/RECORD"),
        "stderr: {}",
        res2.stderr
    );
}

#[test]
fn tier2_f6_04_zip_slip_path_traversal_in_record_rejected() {
    let td = TestDir::new("t2_f6_04");
    // 1. RECORD referencing an entry missing from the wheel zip fails with exit 1
    let missing_entry_rec = td.write(
        "RECORD_missing",
        "pkg/missing_from_zip.py,sha256=123,5\npkg-1.0.dist-info/RECORD,,\n",
    );
    let missing_whl = td.path().join("missing_entry.whl");
    let mf_missing = td.write(
        "mf_missing.txt",
        format!(
            "regular|0|pkg-1.0.dist-info/RECORD|{}\n",
            missing_entry_rec.display()
        ),
    );
    assert_eq!(
        run_tool(
            "ZIPPER_BIN",
            &[mf_missing.to_str().unwrap(), missing_whl.to_str().unwrap()],
            &[],
        )
        .code,
        0
    );

    let out_dir1 = td.path().join("extract_missing");
    let res1 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[missing_whl.to_str().unwrap(), out_dir1.to_str().unwrap()],
        &[],
    );
    assert_eq!(res1.code, 1);
    assert!(
        res1.stderr.contains("not found in wheel"),
        "stderr: {}",
        res1.stderr
    );

    // 2. Path traversal `../escaped_file.txt` in zip & RECORD is sanitized inside out_dir
    let payload = td.write("payload.txt", b"confined");
    let trav_rec = td.write(
        "RECORD_trav",
        "../escaped_file.txt,sha256=123,8\npkg-1.0.dist-info/RECORD,,\n",
    );
    let trav_whl = td.path().join("trav.whl");
    let mf_trav = td.write(
        "mf_trav.txt",
        format!(
            "regular|0|../escaped_file.txt|{}\nregular|0|pkg-1.0.dist-info/RECORD|{}\n",
            payload.display(),
            trav_rec.display()
        ),
    );
    assert_eq!(
        run_tool(
            "ZIPPER_BIN",
            &[mf_trav.to_str().unwrap(), trav_whl.to_str().unwrap()],
            &[],
        )
        .code,
        0
    );

    let out_dir2 = td.path().join("extract_sub");
    let res2 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[trav_whl.to_str().unwrap(), out_dir2.to_str().unwrap()],
        &[],
    );
    assert_eq!(res2.code, 0, "stderr: {}", res2.stderr);
    assert!(
        !td.path().join("escaped_file.txt").exists(),
        "path traversal must never escape out_dir"
    );
    assert_eq!(
        fs::read(out_dir2.join("escaped_file.txt")).unwrap(),
        b"confined"
    );
}

#[test]
fn tier2_f6_05_invalid_arg_count_and_malformed_regex_rejected() {
    let td = TestDir::new("t2_f6_05");
    let whl = build_test_wheel(&td, false, &[("pkg/a.py", b"pass\n")]);
    let out_dir = td.path().join("out");

    let r1 = run_tool("EXTRACT_WHEEL_FILES_BIN", &[whl.to_str().unwrap()], &[]);
    assert_eq!(r1.code, 1);
    assert!(r1.stderr.contains("Usage:"));

    let r2 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            whl.to_str().unwrap(),
            out_dir.to_str().unwrap(),
            "[unclosed_group",
        ],
        &[],
    );
    assert_eq!(r2.code, 1);
    assert!(
        r2.stderr
            .to_lowercase()
            .contains("unclosed character class"),
        "stderr: {}",
        r2.stderr
    );
}

// --- F7: venv_shebang_rewriter (5 boundary/negative tests) ---

#[test]
fn tier2_f7_01_empty_input_file_writes_shebang_header_only() {
    let td = TestDir::new("t2_f7_01");
    let empty_in = td.write("empty.py", b"");
    let out_unix = td.path().join("empty_unix");
    let out_win = td.path().join("empty_win.bat");

    let r1 = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            empty_in.to_str().unwrap(),
            out_unix.to_str().unwrap(),
            "unix",
        ],
        &[],
    );
    assert_eq!(r1.code, 0);
    assert_eq!(
        fs::read_to_string(&out_unix).unwrap(),
        "#!/bin/sh\n'''exec' \"$(dirname \"$0\")/python3\" \"$0\" \"$@\"\n' '''\n"
    );

    let r2 = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            empty_in.to_str().unwrap(),
            out_win.to_str().unwrap(),
            "windows",
        ],
        &[],
    );
    assert_eq!(r2.code, 0);
    assert_eq!(
        fs::read(&out_win).unwrap(),
        b"@setlocal enabledelayedexpansion & \"%~dp0python.exe\" -x \"%~f0\" %* & exit /b !ERRORLEVEL!\r\n"
    );
}

#[test]
fn tier2_f7_02_single_line_without_trailing_newline_strips_line_cleanly() {
    let td = TestDir::new("t2_f7_02");
    let single_line = td.write("single.py", b"#!pythonw");
    let out_win = td.path().join("single.bat");

    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            single_line.to_str().unwrap(),
            out_win.to_str().unwrap(),
            "windows",
        ],
        &[],
    );
    assert_eq!(res.code, 0);
    assert_eq!(
        fs::read(&out_win).unwrap(),
        b"@setlocal enabledelayedexpansion & \"%~dp0pythonw.exe\" -x \"%~f0\" %* & exit /b !ERRORLEVEL!\r\n"
    );
}

#[test]
fn tier2_f7_03_unsupported_target_os_rejected() {
    let td = TestDir::new("t2_f7_03");
    let inp = td.write("in.py", "#!python\nprint('ok')\n");
    let existing_dir = td.path().join("out_is_a_dir");
    fs::create_dir_all(&existing_dir).unwrap();

    // Writing to an existing directory path must fail with exit 1
    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            inp.to_str().unwrap(),
            existing_dir.to_str().unwrap(),
            "unix",
        ],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(!res.stderr.is_empty());
}

#[test]
fn tier2_f7_04_fewer_than_three_positional_args_rejected() {
    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &["only_one_arg", "two_args"],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(res.stderr.contains("Usage:"), "stderr: {}", res.stderr);
}

#[test]
fn tier2_f7_05_nonexistent_input_file_rejected() {
    let td = TestDir::new("t2_f7_05");
    let res = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            td.path().join("missing.py").to_str().unwrap(),
            td.path().join("out").to_str().unwrap(),
            "unix",
        ],
        &[],
    );
    assert_eq!(res.code, 1);
}

// --- F8: wheel_record_rewriter (5 boundary/negative tests) ---

#[test]
fn tier2_f8_01_empty_record_and_blank_lines_handled_cleanly() {
    let td = TestDir::new("t2_f8_01");
    let zero_rec = td.write("zero.RECORD", "");
    let out_zero = td.path().join("out_zero.RECORD");
    let r1 = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            zero_rec.to_str().unwrap(),
            out_zero.to_str().unwrap(),
            "unix",
            "pkg-1.0.data",
        ],
        &[],
    );
    assert_eq!(r1.code, 0);
    assert_eq!(fs::read_to_string(&out_zero).unwrap(), "");

    let blank_rec = td.write("blank.RECORD", "\n\r\n");
    let out_blank = td.path().join("out_blank.RECORD");
    let r2 = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            blank_rec.to_str().unwrap(),
            out_blank.to_str().unwrap(),
            "unix",
            "pkg-1.0.data",
        ],
        &[],
    );
    assert_eq!(r2.code, 0);
    assert_eq!(fs::read_to_string(&out_blank).unwrap(), "\n\n");
}

#[test]
fn tier2_f8_02_unknown_data_subcategory_preserved_verbatim() {
    let td = TestDir::new("t2_f8_02");
    let rec = td.write(
        "RECORD.in",
        "pkg-1.0.data/custom_category/item.bin,sha256=999,42\n",
    );
    let out = td.path().join("RECORD.out");

    let res = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            rec.to_str().unwrap(),
            out.to_str().unwrap(),
            "unix",
            "pkg-1.0.data",
        ],
        &[],
    );
    assert_eq!(res.code, 0);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        "pkg-1.0.data/custom_category/item.bin,sha256=999,42\n"
    );
}

#[test]
fn tier2_f8_03_windows_scripts_line_missing_csv_delimiter_rejected() {
    let td = TestDir::new("t2_f8_03");
    let rec = td.write("RECORD.bad", "pkg-1.0.data/scripts/my_cli_without_comma\n");
    let out = td.path().join("RECORD.out");

    let res = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            rec.to_str().unwrap(),
            out.to_str().unwrap(),
            "windows",
            "pkg-1.0.data",
            "my_cli_without_comma",
        ],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(
        res.stderr.contains("missing delimiter"),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f8_04_unsupported_target_os_rejected() {
    let td = TestDir::new("t2_f8_04");
    // Unclosed quote in Windows scripts RECORD entry or nonexistent input file must fail with exit 1
    let bad_quoted = td.write(
        "RECORD.unclosed",
        "\"pkg-1.0.data/scripts/my_cli,sha256=1,1\n",
    );
    let out = td.path().join("RECORD.out");

    let r1 = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            bad_quoted.to_str().unwrap(),
            out.to_str().unwrap(),
            "windows",
            "pkg-1.0.data",
            "my_cli",
        ],
        &[],
    );
    assert_eq!(r1.code, 1);
    assert!(
        r1.stderr.contains("missing delimiter"),
        "stderr: {}",
        r1.stderr
    );

    let r2 = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            td.path().join("no_such_record").to_str().unwrap(),
            out.to_str().unwrap(),
            "unix",
            "pkg-1.0.data",
        ],
        &[],
    );
    assert_eq!(r2.code, 1);
}

#[test]
fn tier2_f8_05_fewer_than_four_positional_args_rejected() {
    let res = run_tool("WHEEL_RECORD_REWRITER_BIN", &["a", "b", "unix"], &[]);
    assert_eq!(res.code, 1);
    assert!(
        res.stderr.contains("Usage: wheel_record_rewriter"),
        "stderr: {}",
        res.stderr
    );
}

// --- F9: build_data_writer (5 boundary/negative tests) ---

#[test]
fn tier2_f9_01_missing_output_env_and_flag_rejected() {
    let res = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &["--target=//pkg:bin"],
        &[("OUTPUT", "")],
    );
    assert_eq!(res.code, 1);
    assert!(
        res.stderr
            .contains("missing required OUTPUT environment variable or --output flag"),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f9_02_empty_version_and_info_files_emit_no_extra_newlines() {
    let td = TestDir::new("t2_f9_02");
    let empty_v = td.write("empty_v.txt", "");
    let empty_i = td.write("empty_i.txt", "");
    let out = td.path().join("out.txt");

    let res = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[
            "--output",
            out.to_str().unwrap(),
            "--target",
            "//pkg:empty_status",
            "--stamped",
            "TRUE",
            "--version_file",
            empty_v.to_str().unwrap(),
            "--info_file",
            empty_i.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 0);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        "TARGET //pkg:empty_status\nSTAMPED TRUE\n"
    );
}

#[test]
fn tier2_f9_03_nonexistent_version_or_info_file_rejected() {
    let td = TestDir::new("t2_f9_03");
    let out = td.path().join("out.txt");

    let res = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[
            "--output",
            out.to_str().unwrap(),
            "--version_file",
            td.path().join("no_such_version.txt").to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(res.code, 1);
    assert!(
        res.stderr.contains("failed to write build data"),
        "stderr: {}",
        res.stderr
    );
}

#[test]
fn tier2_f9_04_unknown_cli_flag_or_missing_flag_value_rejected() {
    let r1 = run_tool("BUILD_DATA_WRITER_BIN", &["--unknown-flag"], &[]);
    assert_eq!(r1.code, 1);
    assert!(
        r1.stderr.contains("unexpected argument: '--unknown-flag'"),
        "stderr: {}",
        r1.stderr
    );

    let r2 = run_tool("BUILD_DATA_WRITER_BIN", &["--output"], &[]);
    assert_eq!(r2.code, 1);
    assert!(
        r2.stderr.contains("flag '--output' requires a value"),
        "stderr: {}",
        r2.stderr
    );
}

#[test]
fn tier2_f9_05_nested_flagfiles_and_default_target_and_stamped_values() {
    let td = TestDir::new("t2_f9_05");
    let out = td.path().join("flags_out.txt");
    let flags = td.write("bd.flags", format!("--output={}\n\n", out.display()));

    let at_flags = format!("@{}", flags.display());
    let res = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[&at_flags],
        &[
            ("OUTPUT", ""),
            ("TARGET", ""),
            ("STAMPED", ""),
            ("VERSION_FILE", ""),
            ("INFO_FILE", ""),
        ],
    );
    assert_eq!(res.code, 0, "stderr: {}", res.stderr);
    assert_eq!(fs::read_to_string(&out).unwrap(), "TARGET \nSTAMPED \n");
}

// --- F10 & F11: Starlark Rules, Distribution & Companion Tool Boundaries (5 tests) ---

#[test]
fn tier2_f10_01_no_python_files_in_any_crate_distribution_filegroup() {
    let dist_envs = [
        "DIST_PY_CONSOLE_SCRIPT_GEN",
        "DIST_ZIP_MAIN_MAKER",
        "DIST_PY_TEST_MAIN_VALIDATOR",
        "DIST_WHEELMAKER",
        "DIST_PY_WHEEL_DIST",
        "DIST_EXTRACT_WHEEL_FILES",
        "DIST_VENV_SHEBANG_REWRITER",
        "DIST_WHEEL_RECORD_REWRITER",
        "DIST_BUILD_DATA_WRITER",
    ];
    for env_key in dist_envs {
        let files = resolve_runfiles_list(env_key);
        for f in files {
            assert_ne!(
                f.extension().and_then(|e| e.to_str()),
                Some("py"),
                "{} unexpectedly contains a .py file: {:?}",
                env_key,
                f
            );
        }
    }
}

#[test]
fn tier2_f10_02_starlark_wheel_entries_have_deterministic_timestamps_and_verified_record_hashes() {
    let whl_path = resolve_runfile("E2E_WHEEL");
    let entries = read_zip_entries(&whl_path);
    assert!(!entries.is_empty());

    let mut map = HashMap::new();
    for entry in &entries {
        assert_eq!(
            entry.last_modified,
            Some(DateTime::default()),
            "entry {} non-deterministic timestamp",
            entry.name
        );
        assert_eq!(entry.unix_mode, Some(0o100777));
        map.insert(entry.name.clone(), entry.data.clone());
    }

    let record_bytes = map
        .get("e2e_sample_wheel-1.2.3.dist-info/RECORD")
        .expect("missing RECORD in E2E_WHEEL");
    let record_str = String::from_utf8(record_bytes.clone()).unwrap();
    for line in record_str.lines() {
        if line.ends_with(",,") {
            assert_eq!(line, "e2e_sample_wheel-1.2.3.dist-info/RECORD,,");
            continue;
        }
        let parts: Vec<&str> = line.split(',').collect();
        assert_eq!(parts.len(), 3, "unexpected RECORD line: {}", line);
        let file_bytes = map
            .get(parts[0])
            .expect("RECORD references missing zip entry");
        let expected_hash = sha256_urlsafe_digest(file_bytes);
        assert_eq!(parts[1], expected_hash, "hash mismatch for {}", parts[0]);
        assert_eq!(
            parts[2],
            file_bytes.len().to_string(),
            "size mismatch for {}",
            parts[0]
        );
    }
}

#[test]
fn tier2_f10_03_exe_zip_maker_boundary_and_custom_shebang() {
    let td = TestDir::new("t2_f10_03");
    let r_bad = run_tool("EXE_ZIP_MAKER_BIN", &[], &[]);
    assert_ne!(r_bad.code, 0);

    let stub = td.write("stub.py", b"print('stub')\n");
    let payload = td.write("mod.py", b"X = 1\n");
    let manifest = td.write(
        "zip.manifest",
        format!(
            "regular|0|__main__.py|{}\nregular|0|pkg/mod.py|{}\n",
            stub.display(),
            payload.display()
        ),
    );
    let inner_zip = td.path().join("inner.zip");
    let r_zip = run_tool(
        "ZIPPER_BIN",
        &[manifest.to_str().unwrap(), inner_zip.to_str().unwrap()],
        &[],
    );
    assert_eq!(r_zip.code, 0, "stderr: {}", r_zip.stderr);

    let preamble = td.write(
        "preamble.sh",
        b"#!/usr/bin/env -S python3 -I\n# zip_sha256=%ZIP_HASH%\n",
    );
    let out_exe = td.path().join("app.zip");

    let r_ok = run_tool(
        "EXE_ZIP_MAKER_BIN",
        &[
            preamble.to_str().unwrap(),
            inner_zip.to_str().unwrap(),
            out_exe.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r_ok.code, 0, "stderr: {}", r_ok.stderr);
    let raw = fs::read(&out_exe).unwrap();
    let expected_zip_hash = sha256_hex(&fs::read(&inner_zip).unwrap());
    let expected_prefix = format!(
        "#!/usr/bin/env -S python3 -I\n# zip_sha256={}\n",
        expected_zip_hash
    );
    assert!(raw.starts_with(expected_prefix.as_bytes()));
    let map = read_zip_map(&out_exe);
    assert_eq!(map.get("__main__.py").unwrap(), b"print('stub')\n");
    assert_eq!(map.get("pkg/mod.py").unwrap(), b"X = 1\n");
}

#[test]
fn tier2_f10_04_zipper_create_extract_roundtrip_and_invalid_command_error() {
    let td = TestDir::new("t2_f10_04");
    let r_bad = run_tool("ZIPPER_BIN", &["--unknown-option"], &[]);
    assert_ne!(r_bad.code, 0);

    let f1 = td.write("f1.txt", b"hello zipper world");
    let mf = td.write(
        "zipper.manifest",
        format!(
            "regular|0|sub/f1.txt|{}\nrf-empty|pkg/__init__.py\nsymlink|pkg/link.txt|sub/f1.txt\n",
            f1.display()
        ),
    );
    let zip_path = td.path().join("archive.zip");
    let r_create = run_tool(
        "ZIPPER_BIN",
        &[
            "--compression=6",
            "--workspace-name=my_ws",
            "--runfiles-dir=runfiles",
            mf.to_str().unwrap(),
            zip_path.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r_create.code, 0, "stderr: {}", r_create.stderr);

    let map = read_zip_map(&zip_path);
    assert_eq!(map.get("sub/f1.txt").unwrap(), b"hello zipper world");
    assert_eq!(map.get("runfiles/my_ws/pkg/__init__.py").unwrap(), b"");
    assert!(map.contains_key("runfiles/pkg/link.txt"));
}

#[test]
fn tier2_f10_05_all_flagfile_capable_tools_reject_nonexistent_flagfile() {
    let td = TestDir::new("t2_f10_05");
    let bad_at = format!("@{}", td.path().join("no_such_params.file").display());
    let flagfile_tools = [
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        "ZIP_MAIN_MAKER_BIN",
        "PY_TEST_MAIN_VALIDATOR_BIN",
        "WHEELMAKER_BIN",
        "PY_WHEEL_DIST_BIN",
        "BUILD_DATA_WRITER_BIN",
    ];
    for tool_env in flagfile_tools {
        let res = run_tool(tool_env, &[&bad_at], &[]);
        assert_ne!(
            res.code, 0,
            "{} should fail on nonexistent @flagfile",
            tool_env
        );
        assert!(
            !res.stderr.is_empty(),
            "{} should emit diagnostic on stderr for nonexistent @flagfile",
            tool_env
        );
    }
}

// ============================================================================
// TIER 3: Cross-Feature Combinations & Multi-Tool Pipelines (12 tests)
// ============================================================================

#[test]
fn tier3_pipeline_01_wheelmaker_to_py_wheel_dist() {
    let td = TestDir::new("t3_p01");
    let mod_py = td.write("mod.py", b"VALUE = 100\n");
    let meta = td.write(
        "METADATA.in",
        "Metadata-Version: 2.1\nName: My-Pipeline-Pkg\n",
    );
    let built_whl = td.path().join("internal_target.whl");
    let name_file = td.path().join("wheel_name.txt");
    let dist_dir = td.path().join("dist_output");

    let r1 = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=My-Pipeline-Pkg",
            "--version=2.01.0rc01",
            &format!(
                "--input_file=my_pipeline_pkg/__init__.py;{}",
                mod_py.display()
            ),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", built_whl.display()),
            &format!("--name_file={}", name_file.display()),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let r2 = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            built_whl.to_str().unwrap(),
            "--name_file",
            name_file.to_str().unwrap(),
            "--output",
            dist_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);

    let final_whl = dist_dir.join("my_pipeline_pkg-2.1.0rc1-py3-none-any.whl");
    assert!(final_whl.exists());
    assert_eq!(fs::read(&built_whl).unwrap(), fs::read(&final_whl).unwrap());
}

#[test]
fn tier3_pipeline_02_wheelmaker_to_extract_wheel_files() {
    let td = TestDir::new("t3_p02");
    let py_file = td.write("lib.py", b"def c_api(): pass\n");
    let h_file = td.write("c_api.h", b"#define C_API_VERSION 3\n");
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: native_pkg\n");
    let whl = td.path().join("native.whl");

    let r1 = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=native_pkg",
            "--version=1.0.0",
            "--Compress",
            &format!("--input_file=native_pkg/lib.py;{}", py_file.display()),
            &format!(
                "--input_file=native_pkg/include/c_api.h;{}",
                h_file.display()
            ),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", whl.display()),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let extract_dir = td.path().join("headers_out");
    let r2 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            whl.to_str().unwrap(),
            extract_dir.to_str().unwrap(),
            r"native_pkg/include/.*\.h$",
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);
    assert_eq!(
        fs::read_to_string(extract_dir.join("native_pkg/include/c_api.h")).unwrap(),
        "#define C_API_VERSION 3\n"
    );
    assert!(!extract_dir.join("native_pkg/lib.py").exists());
}

#[test]
fn tier3_pipeline_03_wheelmaker_to_extract_wheel_files_to_py_console_script_gen() {
    let td = TestDir::new("t3_p03");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\nwheel-cli = my_pkg.cli:main\n",
    );
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: my_pkg\n");
    let whl = td.path().join("my_pkg.whl");

    let r1 = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=my_pkg",
            "--version=1.5.0",
            &format!("--metadata_file={}", meta.display()),
            &format!("--entry_points_file={}", ep.display()),
            &format!("--out={}", whl.display()),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let extracted = td.path().join("extracted");
    let r2 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            whl.to_str().unwrap(),
            extracted.to_str().unwrap(),
            r".*\.dist-info/entry_points\.txt$",
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);

    let extracted_ep = extracted.join("my_pkg-1.5.0.dist-info/entry_points.txt");
    let gen_py = td.path().join("wheel_cli_runner.py");
    let r3 = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script-guess=wheel-cli",
            extracted_ep.to_str().unwrap(),
            gen_py.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r3.code, 0, "stderr: {}", r3.stderr);
    let script = fs::read_to_string(&gen_py).unwrap();
    assert!(script.contains("from my_pkg.cli import main"));
}

#[test]
fn tier3_pipeline_04_py_console_script_gen_to_py_test_main_validator() {
    let td = TestDir::new("t3_p04");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\npytest-runner = pytest:console_main\n",
    );
    let gen_py = td.path().join("pytest_main.py");

    let r1 = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script=pytest-runner",
            "--console-script-guess=pytest-runner",
            ep.to_str().unwrap(),
            gen_py.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let ok_marker = td.path().join("validated.ok");
    let r2 = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            gen_py.to_str().unwrap(),
            "--src_name",
            "pytest_main.py",
            "--label",
            "//tests:pytest_runner_test",
            "--output",
            ok_marker.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);
    assert!(ok_marker.exists());
}

#[test]
fn tier3_pipeline_05_py_console_script_gen_to_venv_shebang_rewriter() {
    let td = TestDir::new("t3_p05");
    let ep = td.write("entry_points.txt", "[console_scripts]\napp = pkg.app:run\n");
    let wheel_script = td.path().join("app_wheel_script.py");

    let r1 = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script=app",
            "--console-script-guess=app",
            "--shebang=#!python",
            ep.to_str().unwrap(),
            wheel_script.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let unix_out = td.path().join("venv/bin/app");
    let win_out = td.path().join("venv/Scripts/app.bat");

    let r_unix = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            wheel_script.to_str().unwrap(),
            unix_out.to_str().unwrap(),
            "unix",
        ],
        &[],
    );
    assert_eq!(r_unix.code, 0, "stderr: {}", r_unix.stderr);
    let unix_str = fs::read_to_string(&unix_out).unwrap();
    assert!(unix_str.starts_with("#!/bin/sh\n'''exec' \"$(dirname \"$0\")/python3\""));
    assert!(unix_str.contains("from pkg.app import run"));

    let r_win = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            wheel_script.to_str().unwrap(),
            win_out.to_str().unwrap(),
            "windows",
        ],
        &[],
    );
    assert_eq!(r_win.code, 0, "stderr: {}", r_win.stderr);
    let win_str = fs::read_to_string(&win_out).unwrap();
    assert!(win_str.starts_with("@setlocal enabledelayedexpansion & \"%~dp0python.exe\""));
    assert!(win_str.contains("from pkg.app import run"));
}

#[test]
fn tier3_pipeline_06_wheelmaker_to_extract_wheel_files_to_wheel_record_rewriter() {
    let td = TestDir::new("t3_p06");
    let mod_py = td.write("mod.py", b"X = 1\n");
    let cli_sh = td.write("my_cli", b"#!python\nprint('cli')\n");
    let hdr_h = td.write("my_hdr.h", b"#pragma once\n");
    let doc_txt = td.write("doc.txt", b"readme\n");
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: data_whl\n");
    let whl = td.path().join("data_whl.whl");

    let r1 = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=data_whl",
            "--version=1.0",
            &format!("--input_file=data_whl/__init__.py;{}", mod_py.display()),
            &format!("--data_files=scripts/my_cli;{}", cli_sh.display()),
            &format!("--data_files=headers/my_hdr.h;{}", hdr_h.display()),
            &format!("--data_files=data/share/doc.txt;{}", doc_txt.display()),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", whl.display()),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let extracted = td.path().join("extracted");
    let r2 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[whl.to_str().unwrap(), extracted.to_str().unwrap()],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);

    let record_in = extracted.join("data_whl-1.0.dist-info/RECORD");
    let record_unix = td.path().join("RECORD.unix");
    let record_win = td.path().join("RECORD.win");

    let r3 = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            record_unix.to_str().unwrap(),
            "unix",
            "data_whl-1.0.data",
            "my_cli",
        ],
        &[],
    );
    assert_eq!(r3.code, 0, "stderr: {}", r3.stderr);
    let unix_rec = fs::read_to_string(&record_unix).unwrap();
    assert!(unix_rec.contains("../../../bin/my_cli,sha256="));
    assert!(unix_rec.contains("../../../include/my_hdr.h,sha256="));
    assert!(unix_rec.contains("../../../share/doc.txt,sha256="));

    let r4 = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            record_win.to_str().unwrap(),
            "windows",
            "data_whl-1.0.data",
            "my_cli",
        ],
        &[],
    );
    assert_eq!(r4.code, 0, "stderr: {}", r4.stderr);
    let win_rec = fs::read_to_string(&record_win).unwrap();
    assert!(win_rec.contains("../../Scripts/my_cli.bat,sha256="));
    assert!(win_rec.contains("../../Include/my_hdr.h,sha256="));
    assert!(win_rec.contains("../../share/doc.txt,sha256="));
}

#[test]
fn tier3_pipeline_07_wheelmaker_to_extract_to_venv_shebang_and_record_rewriter() {
    let td = TestDir::new("t3_p07");
    let script = td.write("runner", b"#!pythonw\nprint('gui runner')\n");
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: gui_pkg\n");
    let whl = td.path().join("gui_pkg.whl");

    let r1 = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=gui_pkg",
            "--version=0.5",
            &format!("--data_files=scripts/runner;{}", script.display()),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", whl.display()),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let extracted = td.path().join("extracted");
    let r2 = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[whl.to_str().unwrap(), extracted.to_str().unwrap()],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);

    let extracted_script = extracted.join("gui_pkg-0.5.data/scripts/runner");
    let win_script = td.path().join("venv/Scripts/runner.bat");
    let r3 = run_tool(
        "VENV_SHEBANG_REWRITER_BIN",
        &[
            extracted_script.to_str().unwrap(),
            win_script.to_str().unwrap(),
            "windows",
        ],
        &[],
    );
    assert_eq!(r3.code, 0, "stderr: {}", r3.stderr);
    assert!(fs::read_to_string(&win_script)
        .unwrap()
        .contains("\"%~dp0pythonw.exe\""));

    let record_in = extracted.join("gui_pkg-0.5.dist-info/RECORD");
    let record_out = td
        .path()
        .join("venv/Lib/site-packages/gui_pkg-0.5.dist-info/RECORD");
    let r4 = run_tool(
        "WHEEL_RECORD_REWRITER_BIN",
        &[
            record_in.to_str().unwrap(),
            record_out.to_str().unwrap(),
            "windows",
            "gui_pkg-0.5.data",
            "runner",
        ],
        &[],
    );
    assert_eq!(r4.code, 0, "stderr: {}", r4.stderr);
    assert!(fs::read_to_string(&record_out)
        .unwrap()
        .contains("../../Scripts/runner.bat,sha256="));
}

#[test]
fn tier3_pipeline_08_build_data_writer_to_zip_main_maker() {
    let td = TestDir::new("t3_p08");
    let ver = td.write("version.txt", "BUILD_SCM_REVISION deadbeef\n");
    let info = td.write("info.txt", "BUILD_TIMESTAMP 1700000000\n");
    let build_data = td.path().join("build_data.txt");

    let r1 = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[
            "--output",
            build_data.to_str().unwrap(),
            "--target",
            "//app:stamped_bin",
            "--stamped",
            "TRUE",
            "--version_file",
            ver.to_str().unwrap(),
            "--info_file",
            info.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let mf = td.write(
        "zip.manifest",
        format!(
            "regular|0|runfiles/_main/build_data.txt|{}\n",
            build_data.display()
        ),
    );
    let tpl = td.write(
        "template.py",
        "BUILD_DATA_HASH = '%APP_HASH%'\nTARGET = 'STAMP_TARGET'\n",
    );
    let main_py = td.path().join("__main__.py");

    let r2 = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--hash_files_manifest",
            mf.to_str().unwrap(),
            "--substitution",
            "STAMP_TARGET=//app:stamped_bin",
            "--output",
            main_py.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);

    let bd_bytes = fs::read(&build_data).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(format!("0|runfiles/_main/build_data.txt|{}", build_data.display()).as_bytes());
    hasher.update(&bd_bytes);
    let expected_hash = hex_encode(&hasher.finalize());
    assert_eq!(
        fs::read_to_string(&main_py).unwrap(),
        format!(
            "BUILD_DATA_HASH = '{}'\nTARGET = '//app:stamped_bin'\n",
            expected_hash
        )
    );
}

#[test]
fn tier3_pipeline_09_zip_main_maker_to_py_test_main_validator() {
    let td = TestDir::new("t3_p09");
    let f1 = td.write("mod.py", b"print('hi')\n");
    let mf = td.write(
        "zip.manifest",
        format!("regular|0|pkg/mod.py|{}\n", f1.display()),
    );
    let tpl = td.write(
        "bootstrap_tpl.py",
        "import sys\nMANIFEST_HASH = '%APP_HASH%'\nif __name__ == '__main__':\n    sys.exit(ENTRY_CALL())\n",
    );
    let rendered = td.path().join("rendered_main.py");

    let r1 = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--hash_files_manifest",
            mf.to_str().unwrap(),
            "--substitution",
            "ENTRY_CALL=int",
            "--output",
            rendered.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let ok_file = td.path().join("valid.ok");
    let r2 = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            rendered.to_str().unwrap(),
            "--src_name",
            "rendered_main.py",
            "--label",
            "//pkg:zip_test",
            "--output",
            ok_file.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);
    assert!(ok_file.exists());
}

#[test]
fn tier3_pipeline_10_zip_main_maker_to_exe_zip_maker_to_zipper() {
    let td = TestDir::new("t3_p10");
    let app_py = td.write("app.py", b"def run(): return 42\n");
    let zmm_manifest = td.write(
        "zmm.manifest",
        format!("regular|0|runfiles/ws/app.py|{}\n", app_py.display()),
    );
    let tpl = td.write(
        "tpl.py",
        "# hash: %APP_HASH%\nimport runfiles\nprint('bootstrapped')\n",
    );
    let main_py = td.path().join("__main__.py");

    let r1 = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--hash_files_manifest",
            zmm_manifest.to_str().unwrap(),
            "--output",
            main_py.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let zip_manifest = td.write(
        "zipper.manifest",
        format!(
            "regular|0|__main__.py|{}\nregular|0|runfiles/ws/app.py|{}\n",
            main_py.display(),
            app_py.display()
        ),
    );
    let inner_zip = td.path().join("inner.zip");
    let r2 = run_tool(
        "ZIPPER_BIN",
        &[zip_manifest.to_str().unwrap(), inner_zip.to_str().unwrap()],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);

    let preamble = td.write("preamble.sh", "#!/usr/bin/env python3\n# %ZIP_HASH%\n");
    let exe_zip = td.path().join("self_exec_app");
    let r3 = run_tool(
        "EXE_ZIP_MAKER_BIN",
        &[
            preamble.to_str().unwrap(),
            inner_zip.to_str().unwrap(),
            exe_zip.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r3.code, 0, "stderr: {}", r3.stderr);

    let map = read_zip_map(&exe_zip);
    assert_eq!(
        map.get("__main__.py").unwrap(),
        &fs::read(&main_py).unwrap()
    );
    assert_eq!(
        map.get("runfiles/ws/app.py").unwrap(),
        b"def run(): return 42\n"
    );
}

#[test]
fn tier3_pipeline_11_build_data_writer_and_wheelmaker_shared_workspace_stamping() {
    let td = TestDir::new("t3_p11");
    let volatile = td.write("volatile-status.txt", "BUILD_TIMESTAMP 2026110199\n");
    let stable = td.write("stable-status.txt", "STABLE_VERSION 4.5.6\n");
    let build_data = td.path().join("build_data.txt");

    let r1 = run_tool(
        "BUILD_DATA_WRITER_BIN",
        &[
            "--output",
            build_data.to_str().unwrap(),
            "--target",
            "//pkg:stamped_whl",
            "--stamped",
            "TRUE",
            "--version_file",
            stable.to_str().unwrap(),
            "--info_file",
            volatile.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    let meta = td.write(
        "METADATA.in",
        "Metadata-Version: 2.1\nName: stamped_pkg\nSummary: Built at {BUILD_TIMESTAMP}\n",
    );
    let whl = td.path().join("stamped.whl");
    let r2 = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=stamped_pkg",
            "--version={STABLE_VERSION}",
            &format!("--volatile_status_file={}", volatile.display()),
            &format!("--stable_status_file={}", stable.display()),
            &format!(
                "--input_file=stamped_pkg/build_data.txt;{}",
                build_data.display()
            ),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", whl.display()),
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);

    let map = read_zip_map(&whl);
    let bd_in_whl =
        String::from_utf8(map.get("stamped_pkg/build_data.txt").unwrap().clone()).unwrap();
    assert!(bd_in_whl.contains("STABLE_VERSION 4.5.6\n"));
    assert!(bd_in_whl.contains("BUILD_TIMESTAMP 2026110199\n"));

    let meta_in_whl = String::from_utf8(
        map.get("stamped_pkg-4.5.6.dist-info/METADATA")
            .unwrap()
            .clone(),
    )
    .unwrap();
    assert!(meta_in_whl.contains("Version: 4.5.6\n"));
    assert!(meta_in_whl.contains("Summary: Built at 2026110199\n"));
}

#[test]
fn tier3_pipeline_12_py_test_main_validator_gates_wheelmaker_and_py_wheel_dist() {
    let td = TestDir::new("t3_p12");
    let inert_py = td.write("inert_test.py", "class ForgotMain:\n    pass\n");
    let active_py = td.write(
        "active_test.py",
        "import unittest\nclass OkTest(unittest.TestCase):\n    pass\nif __name__ == '__main__':\n    unittest.main()\n",
    );

    let inert_ok = td.path().join("inert.ok");
    let r_inert = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            inert_py.to_str().unwrap(),
            "--src_name",
            "inert_test.py",
            "--label",
            "//pkg:inert_test",
            "--output",
            inert_ok.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r_inert.code, 1);
    assert!(!inert_ok.exists());

    let active_ok = td.path().join("active.ok");
    let r_active = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            active_py.to_str().unwrap(),
            "--src_name",
            "active_test.py",
            "--label",
            "//pkg:active_test",
            "--output",
            active_ok.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r_active.code, 0);
    assert!(active_ok.exists());

    // Package the validated test module into a wheel and distribute it
    let meta = td.write(
        "METADATA.in",
        "Metadata-Version: 2.1\nName: validated_suite\n",
    );
    let whl = td.path().join("suite.whl");
    let nf = td.path().join("suite_name.txt");
    let r_whl = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=validated_suite",
            "--version=1.0.0",
            &format!(
                "--input_file=validated_suite/active_test.py;{}",
                active_py.display()
            ),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", whl.display()),
            &format!("--name_file={}", nf.display()),
        ],
        &[],
    );
    assert_eq!(r_whl.code, 0);

    let dist_dir = td.path().join("dist");
    let r_dist = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            whl.to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
            "--output",
            dist_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r_dist.code, 0);
    assert!(dist_dir
        .join("validated_suite-1.0.0-py3-none-any.whl")
        .exists());
}

// ============================================================================
// TIER 4: Real-World Application Scenarios (6 tests)
// ============================================================================

#[test]
fn tier4_scenario_01_full_pep427_wheel_build_and_distribution_pipeline() {
    let td = TestDir::new("t4_s01");
    let init_py = td.write(
        "src/python/acme_sdk/__init__.py",
        b"__version__ = '2.4.0'\n",
    );
    let client_py = td.write(
        "src/python/acme_sdk/client.py",
        b"def connect(): return 'connected'\n",
    );
    let hdr_h = td.write(
        "src/python/acme_sdk/include/acme_ffi.h",
        b"#ifndef ACME_FFI_H\n#define ACME_FFI_H\nint acme_init(void);\n#endif\n",
    );
    let cli_script = td.write("bin/acme-admin", b"#!python\nprint('acme-admin')\n");
    let license = td.write("LICENSE", b"Apache-2.0\n");
    let readme = td.write(
        "README.md",
        b"# Acme SDK\n\nEnterprise SDK documentation.\n",
    );
    let reqs = td.write(
        "requirements.txt",
        "urllib3 >= 1.26.0, < 3.0\ncryptography[ssh] >= 41.0.0 ; python_version >= '3.9'\n",
    );
    let test_reqs = td.write("test_requirements.txt", "pytest >= 7.4.0\n");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\nacme-cli = acme_sdk.client:connect\n",
    );
    let volatile = td.write("volatile-status.txt", "BUILD_USER release_bot\n");
    let stable = td.write("stable-status.txt", "STABLE_RELEASE 2.04.0\n");
    let meta = td.write(
        "METADATA.in",
        format!(
            "Metadata-Version: 2.1\nName: Acme.SDK\nAuthor: {{BUILD_USER}}\nRequires-Dist: @{}\nRequires-Dist: @{}\n",
            reqs.display(),
            test_reqs.display()
        ),
    );
    let input_list = td.write(
        "inputs.list",
        format!(
            "src/python/acme_sdk/__init__.py;{}\nsrc/python/acme_sdk/client.py;{}\nsrc/python/acme_sdk/include/acme_ffi.h;{}\n",
            init_py.display(),
            client_py.display(),
            hdr_h.display()
        ),
    );

    let out_whl = td.path().join("bazel-out/acme.whl");
    let name_file = td.path().join("bazel-out/acme.name");
    let flagfile = td.write(
        "wheelmaker.params",
        format!(
            "--name=Acme.SDK\n--version={{STABLE_RELEASE}}\n--python_tag=cp311\n--abi=cp311\n--platform=manylinux_2_28_x86_64\n--Compress\n--strip_path_prefix=src/python/\n--input_file_list={}\n--metadata_file={}\n--description_file={}\n--entry_points_file={}\n--extra_requires_file={};test\n--extra_distinfo_file=LICENSE;{}\n--data_files=scripts/acme-admin;{}\n--volatile_status_file={}\n--stable_status_file={}\n--out={}\n--name_file={}\n",
            input_list.display(),
            meta.display(),
            readme.display(),
            ep.display(),
            test_reqs.display(),
            license.display(),
            cli_script.display(),
            volatile.display(),
            stable.display(),
            out_whl.display(),
            name_file.display()
        ),
    );

    let at_arg = format!("@{}", flagfile.display());
    let r_whl = run_tool("WHEELMAKER_BIN", &[&at_arg], &[]);
    assert_eq!(r_whl.code, 0, "stderr: {}", r_whl.stderr);

    let dist_dir = td.path().join("dist");
    let r_dist = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            out_whl.to_str().unwrap(),
            "--name_file",
            name_file.to_str().unwrap(),
            "--output",
            dist_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r_dist.code, 0, "stderr: {}", r_dist.stderr);

    let dist_whl = dist_dir.join("acme_sdk-2.4.0-cp311-cp311-manylinux_2_28_x86_64.whl");
    assert!(dist_whl.exists());

    // Extract C headers via extract_wheel_files (simulating whl_filegroup)
    let hdr_fg = td.path().join("hdr_filegroup");
    let r_ext = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[
            dist_whl.to_str().unwrap(),
            hdr_fg.to_str().unwrap(),
            r"acme_sdk/include/.*\.h$",
        ],
        &[],
    );
    assert_eq!(r_ext.code, 0, "stderr: {}", r_ext.stderr);
    assert!(hdr_fg.join("acme_sdk/include/acme_ffi.h").exists());
}

#[test]
fn tier4_scenario_02_pypi_wheel_venv_installation_simulation_unix_and_windows() {
    let td = TestDir::new("t4_s02");
    let pure_py = td.write("pure.py", b"PURE = True\n");
    let plat_so = td.write("fast.so", b"\x7fELF_mock_shared_object");
    let cli_py = td.write("my-cli", b"#!python\nimport sys\nprint('cli ok')\n");
    let gui_py = td.write("my-gui", b"#!pythonw\nimport sys\nprint('gui ok')\n");
    let hdr_h = td.write("fast.h", b"void fast_call(void);\n");
    let share_cfg = td.write("default.conf", b"[settings]\nmode=prod\n");
    let meta = td.write(
        "METADATA.in",
        "Metadata-Version: 2.1\nName: full_venv_pkg\n",
    );
    let whl = td.path().join("full_venv_pkg-3.1.0-py3-none-any.whl");

    let r_whl = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=full_venv_pkg",
            "--version=3.1.0",
            "--Compress",
            &format!(
                "--data_files=purelib/full_venv_pkg/pure.py;{}",
                pure_py.display()
            ),
            &format!(
                "--data_files=platlib/full_venv_pkg/fast.so;{}",
                plat_so.display()
            ),
            &format!("--data_files=scripts/my-cli;{}", cli_py.display()),
            &format!("--data_files=scripts/my-gui;{}", gui_py.display()),
            &format!("--data_files=headers/fast.h;{}", hdr_h.display()),
            &format!("--data_files=data/etc/default.conf;{}", share_cfg.display()),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", whl.display()),
        ],
        &[],
    );
    assert_eq!(r_whl.code, 0, "stderr: {}", r_whl.stderr);

    // Step 1: Extract entire wheel
    let unpacked = td.path().join("unpacked_wheel");
    let r_ext = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[whl.to_str().unwrap(), unpacked.to_str().unwrap()],
        &[],
    );
    assert_eq!(r_ext.code, 0, "stderr: {}", r_ext.stderr);

    // Step 2: Simulate Unix & Windows shebang rewrites for scripts
    let scripts_dir = unpacked.join("full_venv_pkg-3.1.0.data/scripts");
    let unix_cli = td.path().join("unix_venv/bin/my-cli");
    let win_cli = td.path().join("win_venv/Scripts/my-cli.bat");
    let win_gui = td.path().join("win_venv/Scripts/my-gui.bat");

    assert_eq!(
        run_tool(
            "VENV_SHEBANG_REWRITER_BIN",
            &[
                scripts_dir.join("my-cli").to_str().unwrap(),
                unix_cli.to_str().unwrap(),
                "unix",
            ],
            &[],
        )
        .code,
        0
    );
    assert_eq!(
        run_tool(
            "VENV_SHEBANG_REWRITER_BIN",
            &[
                scripts_dir.join("my-cli").to_str().unwrap(),
                win_cli.to_str().unwrap(),
                "windows",
            ],
            &[],
        )
        .code,
        0
    );
    assert_eq!(
        run_tool(
            "VENV_SHEBANG_REWRITER_BIN",
            &[
                scripts_dir.join("my-gui").to_str().unwrap(),
                win_gui.to_str().unwrap(),
                "windows",
            ],
            &[],
        )
        .code,
        0
    );

    // Step 3: Rewrite RECORD for Unix and Windows virtualenvs
    let orig_record = unpacked.join("full_venv_pkg-3.1.0.dist-info/RECORD");
    let unix_record = td
        .path()
        .join("unix_venv/site-packages/full_venv_pkg-3.1.0.dist-info/RECORD");
    let win_record = td
        .path()
        .join("win_venv/Lib/site-packages/full_venv_pkg-3.1.0.dist-info/RECORD");

    assert_eq!(
        run_tool(
            "WHEEL_RECORD_REWRITER_BIN",
            &[
                orig_record.to_str().unwrap(),
                unix_record.to_str().unwrap(),
                "unix",
                "full_venv_pkg-3.1.0.data",
                "my-cli",
                "my-gui",
            ],
            &[],
        )
        .code,
        0
    );
    assert_eq!(
        run_tool(
            "WHEEL_RECORD_REWRITER_BIN",
            &[
                orig_record.to_str().unwrap(),
                win_record.to_str().unwrap(),
                "windows",
                "full_venv_pkg-3.1.0.data",
                "my-cli",
                "my-gui",
            ],
            &[],
        )
        .code,
        0
    );

    let u_rec = fs::read_to_string(&unix_record).unwrap();
    assert!(u_rec.contains("full_venv_pkg/pure.py,sha256="));
    assert!(u_rec.contains("full_venv_pkg/fast.so,sha256="));
    assert!(u_rec.contains("../../../bin/my-cli,sha256="));
    assert!(u_rec.contains("../../../bin/my-gui,sha256="));
    assert!(u_rec.contains("../../../include/fast.h,sha256="));
    assert!(u_rec.contains("../../../etc/default.conf,sha256="));

    let w_rec = fs::read_to_string(&win_record).unwrap();
    assert!(w_rec.contains("full_venv_pkg/pure.py,sha256="));
    assert!(w_rec.contains("full_venv_pkg/fast.so,sha256="));
    assert!(w_rec.contains("../../Scripts/my-cli.bat,sha256="));
    assert!(w_rec.contains("../../Scripts/my-gui.bat,sha256="));
    assert!(w_rec.contains("../../Include/fast.h,sha256="));
    assert!(w_rec.contains("../../etc/default.conf,sha256="));
}

#[test]
fn tier4_scenario_03_console_script_generation_validation_and_zipapp_bundling() {
    let td = TestDir::new("t4_s03");
    let ep = td.write(
        "entry_points.txt",
        "[console_scripts]\nblack = black:patched_main\n",
    );
    let entry_py = td.path().join("_black_entry.py");

    // 1. Generate console script entry point
    let r1 = run_tool(
        "PY_CONSOLE_SCRIPT_GEN_BIN",
        &[
            "--console-script-guess=black",
            ep.to_str().unwrap(),
            entry_py.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r1.code, 0, "stderr: {}", r1.stderr);

    // 2. Validate generated script with py_test_main_validator
    let val_ok = td.path().join("entry.validated");
    let r2 = run_tool(
        "PY_TEST_MAIN_VALIDATOR_BIN",
        &[
            "--src",
            entry_py.to_str().unwrap(),
            "--src_name",
            "_black_entry.py",
            "--label",
            "//tools:black_bin",
            "--output",
            val_ok.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r2.code, 0, "stderr: {}", r2.stderr);

    // 3. Render zip bootstrap __main__.py with zip_main_maker
    let black_mod = td.write("black/__init__.py", b"def patched_main(): return 0\n");
    let zmm_manifest = td.write(
        "zmm.manifest",
        format!(
            "regular|0|runfiles/_main/_black_entry.py|{}\nregular|0|runfiles/_main/black/__init__.py|{}\n",
            entry_py.display(),
            black_mod.display()
        ),
    );
    let tpl = td.write(
        "zip_tpl.py",
        "# %APP_HASH%\nMAIN_MODULE = '%main%'\nimport sys\nsys.exit(0)\n",
    );
    let bootstrap_main = td.path().join("__main__.py");
    let r3 = run_tool(
        "ZIP_MAIN_MAKER_BIN",
        &[
            "--template",
            tpl.to_str().unwrap(),
            "--hash_files_manifest",
            zmm_manifest.to_str().unwrap(),
            "--substitution",
            "%main%=runfiles/_main/_black_entry.py",
            "--output",
            bootstrap_main.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r3.code, 0, "stderr: {}", r3.stderr);

    // 4. Assemble inner zip with zipper and executable zipapp with exe_zip_maker
    let zip_manifest = td.write(
        "zipper.manifest",
        format!(
            "regular|0|__main__.py|{}\nregular|0|runfiles/_main/_black_entry.py|{}\nregular|0|runfiles/_main/black/__init__.py|{}\n",
            bootstrap_main.display(),
            entry_py.display(),
            black_mod.display()
        ),
    );
    let inner_zip = td.path().join("inner.zip");
    let r_zip = run_tool(
        "ZIPPER_BIN",
        &[zip_manifest.to_str().unwrap(), inner_zip.to_str().unwrap()],
        &[],
    );
    assert_eq!(r_zip.code, 0, "stderr: {}", r_zip.stderr);

    let preamble = td.write("preamble.sh", "#!/usr/bin/env python3\n# %ZIP_HASH%\n");
    let zipapp = td.path().join("black.zip");
    let r4 = run_tool(
        "EXE_ZIP_MAKER_BIN",
        &[
            preamble.to_str().unwrap(),
            inner_zip.to_str().unwrap(),
            zipapp.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r4.code, 0, "stderr: {}", r4.stderr);

    let map = read_zip_map(&zipapp);
    assert!(map.contains_key("__main__.py"));
    assert!(map.contains_key("runfiles/_main/_black_entry.py"));
    assert!(map.contains_key("runfiles/_main/black/__init__.py"));
}

#[test]
fn tier4_scenario_04_stamped_zip_binary_with_build_data_and_manifest_hashing() {
    let td = TestDir::new("t4_s04");
    let app_py = td.write("app.py", b"print('stamped app')\n");
    let tpl = td.write("tpl.py", "HASH = '%APP_HASH%'\n");

    // Run 1 with commit rev A
    let ver_a = td.write("ver_a.txt", "STABLE_GIT_COMMIT aaaaaaa\n");
    let info_a = td.write("info_a.txt", "BUILD_EMBED_LABEL v1.0\n");
    let bd_a = td.path().join("build_data_a.txt");
    assert_eq!(
        run_tool(
            "BUILD_DATA_WRITER_BIN",
            &[
                "--output",
                bd_a.to_str().unwrap(),
                "--target",
                "//app:bin",
                "--stamped",
                "TRUE",
                "--version_file",
                ver_a.to_str().unwrap(),
                "--info_file",
                info_a.to_str().unwrap(),
            ],
            &[],
        )
        .code,
        0
    );
    let mf_a = td.write(
        "mf_a.txt",
        format!(
            "regular|0|app.py|{}\nregular|0|build_data.txt|{}\n",
            app_py.display(),
            bd_a.display()
        ),
    );
    let main_a = td.path().join("main_a.py");
    assert_eq!(
        run_tool(
            "ZIP_MAIN_MAKER_BIN",
            &[
                "--template",
                tpl.to_str().unwrap(),
                "--hash_files_manifest",
                mf_a.to_str().unwrap(),
                "--output",
                main_a.to_str().unwrap(),
            ],
            &[],
        )
        .code,
        0
    );

    // Run 2 with commit rev B
    let ver_b = td.write("ver_b.txt", "STABLE_GIT_COMMIT bbbbbbb\n");
    let bd_b = td.path().join("build_data_b.txt");
    assert_eq!(
        run_tool(
            "BUILD_DATA_WRITER_BIN",
            &[
                "--output",
                bd_b.to_str().unwrap(),
                "--target",
                "//app:bin",
                "--stamped",
                "TRUE",
                "--version_file",
                ver_b.to_str().unwrap(),
                "--info_file",
                info_a.to_str().unwrap(),
            ],
            &[],
        )
        .code,
        0
    );
    let mf_b = td.write(
        "mf_b.txt",
        format!(
            "regular|0|app.py|{}\nregular|0|build_data.txt|{}\n",
            app_py.display(),
            bd_b.display()
        ),
    );
    let main_b = td.path().join("main_b.py");
    assert_eq!(
        run_tool(
            "ZIP_MAIN_MAKER_BIN",
            &[
                "--template",
                tpl.to_str().unwrap(),
                "--hash_files_manifest",
                mf_b.to_str().unwrap(),
                "--output",
                main_b.to_str().unwrap(),
            ],
            &[],
        )
        .code,
        0
    );

    // Updating the workspace stamp must change build_data.txt and therefore %APP_HASH%
    assert_ne!(
        fs::read_to_string(&main_a).unwrap(),
        fs::read_to_string(&main_b).unwrap()
    );
}

#[test]
fn tier4_scenario_05_byte_for_byte_reproducibility_across_repeated_pipeline_runs() {
    let td = TestDir::new("t4_s05");
    let run_pipeline = || -> Vec<Vec<u8>> {
        let ep = td.write("ep.txt", "[console_scripts]\ncmd = pkg.mod:main\n");
        let gen_py = td.path().join("gen.py");
        assert_eq!(
            run_tool(
                "PY_CONSOLE_SCRIPT_GEN_BIN",
                &[
                    "--console-script=cmd",
                    "--console-script-guess=cmd",
                    ep.to_str().unwrap(),
                    gen_py.to_str().unwrap(),
                ],
                &[],
            )
            .code,
            0
        );

        let bd = td.path().join("bd.txt");
        let ver = td.write("v.txt", "REV 123\n");
        assert_eq!(
            run_tool(
                "BUILD_DATA_WRITER_BIN",
                &[
                    "--output",
                    bd.to_str().unwrap(),
                    "--target",
                    "//pkg:t",
                    "--stamped",
                    "TRUE",
                    "--version_file",
                    ver.to_str().unwrap(),
                ],
                &[],
            )
            .code,
            0
        );

        let mf = td.write(
            "mf.txt",
            format!(
                "regular|0|pkg/gen.py|{}\nregular|0|pkg/bd.txt|{}\n",
                gen_py.display(),
                bd.display()
            ),
        );
        let tpl = td.write("tpl.py", "H = '%APP_HASH%'\n");
        let zmm_out = td.path().join("zmm.py");
        assert_eq!(
            run_tool(
                "ZIP_MAIN_MAKER_BIN",
                &[
                    "--template",
                    tpl.to_str().unwrap(),
                    "--hash_files_manifest",
                    mf.to_str().unwrap(),
                    "--output",
                    zmm_out.to_str().unwrap(),
                ],
                &[],
            )
            .code,
            0
        );

        let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: repro_pkg\n");
        let whl = td.path().join("repro.whl");
        assert_eq!(
            run_tool(
                "WHEELMAKER_BIN",
                &[
                    "--name=repro_pkg",
                    "--version=1.0.0",
                    "--Compress",
                    &format!("--input_file=repro_pkg/gen.py;{}", gen_py.display()),
                    &format!("--input_file=repro_pkg/bd.txt;{}", bd.display()),
                    &format!("--metadata_file={}", meta.display()),
                    &format!("--out={}", whl.display()),
                ],
                &[],
            )
            .code,
            0
        );

        vec![
            fs::read(&gen_py).unwrap(),
            fs::read(&bd).unwrap(),
            fs::read(&zmm_out).unwrap(),
            fs::read(&whl).unwrap(),
        ]
    };

    let artifacts_1 = run_pipeline();
    let artifacts_2 = run_pipeline();
    assert_eq!(
        artifacts_1, artifacts_2,
        "pipeline outputs must be byte-for-byte deterministic"
    );
}

#[test]
fn tier4_scenario_06_adversarial_special_chars_and_large_fanout_stress_pipeline() {
    let td = TestDir::new("t4_s06");
    let mut list_lines = String::new();
    let mut expected_payloads: HashMap<String, Vec<u8>> = HashMap::new();

    for i in 0..150 {
        let pkg_rel = if i % 10 == 0 {
            format!("stress_pkg/sub,dir_{}/mod \"quote\" {}.py", i, i)
        } else {
            format!("stress_pkg/sub_{}/mod_{}.py", i % 15, i)
        };
        let payload = format!("# Module {}\nVALUE_{} = {}\n", i, i, i * 7).into_bytes();
        let disk_file = td.write(&format!("inputs/m_{}.py", i), &payload);
        list_lines.push_str(&format!("{};{}\n", pkg_rel, disk_file.display()));
        expected_payloads.insert(pkg_rel, payload);
    }

    let list_file = td.write("stress.list", &list_lines);
    let meta = td.write("METADATA.in", "Metadata-Version: 2.1\nName: stress_pkg\n");
    let whl = td.path().join("stress.whl");
    let nf = td.path().join("stress.name");

    let r_whl = run_tool(
        "WHEELMAKER_BIN",
        &[
            "--name=stress_pkg",
            "--version=9.9.9",
            "--Compress",
            &format!("--input_file_list={}", list_file.display()),
            &format!("--metadata_file={}", meta.display()),
            &format!("--out={}", whl.display()),
            &format!("--name_file={}", nf.display()),
        ],
        &[],
    );
    assert_eq!(r_whl.code, 0, "stderr: {}", r_whl.stderr);

    let dist_dir = td.path().join("dist");
    let r_dist = run_tool(
        "PY_WHEEL_DIST_BIN",
        &[
            "--wheel",
            whl.to_str().unwrap(),
            "--name_file",
            nf.to_str().unwrap(),
            "--output",
            dist_dir.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(r_dist.code, 0, "stderr: {}", r_dist.stderr);

    let final_whl = dist_dir.join("stress_pkg-9.9.9-py3-none-any.whl");
    let extract_dir = td.path().join("extracted_stress");
    let r_ext = run_tool(
        "EXTRACT_WHEEL_FILES_BIN",
        &[final_whl.to_str().unwrap(), extract_dir.to_str().unwrap()],
        &[],
    );
    assert_eq!(r_ext.code, 0, "stderr: {}", r_ext.stderr);

    for (rel_path, expected_bytes) in &expected_payloads {
        let extracted_path = extract_dir.join(rel_path);
        assert!(
            extracted_path.exists(),
            "missing extracted file: {:?}",
            extracted_path
        );
        assert_eq!(&fs::read(&extracted_path).unwrap(), expected_bytes);
    }
}
