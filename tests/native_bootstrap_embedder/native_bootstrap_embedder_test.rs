use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicUsize, Ordering};

use native_bootstrap_embedder_lib::{
    Cli, Options, RuntimeVenvSymlink, embed_bootstrap_file, embed_config_into_binary, parse_args,
    run_cli, serialize_cfg_payload,
};
use stage1_native_bootstrap_lib::{
    BootstrapConfig, CFG_BUFFER_SIZE, CFG_HEADER_LEN, CFG_MAGIC, CFG_MAGIC_LEN, find_magic_offset,
    load_eof_trailer_config, parse_cfg_payload, parse_eof_trailer_from_bytes,
};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn make_mock_template_binary() -> Vec<u8> {
    let mut bin = Vec::with_capacity(128 + CFG_BUFFER_SIZE + 64);
    bin.extend_from_slice(&[0x7f, b'E', b'L', b'F', 1, 2, 3, 4]);
    bin.resize(128, 0xAA);
    let magic_start = bin.len();
    bin.resize(magic_start + CFG_BUFFER_SIZE, 0);
    bin[magic_start..magic_start + CFG_MAGIC_LEN].copy_from_slice(&CFG_MAGIC);
    bin.extend_from_slice(&[0xBB; 64]);
    bin
}

fn make_mock_windows_template_binary() -> Vec<u8> {
    let mut bin = Vec::with_capacity(128);
    bin.extend_from_slice(b"MZ");
    bin.resize(64, 0xAA);
    bin.extend_from_slice(&CFG_MAGIC);
    bin.extend_from_slice(&[0xBB; 32]);
    bin
}

fn unwrap_run_options(cli: Cli) -> Options {
    match cli {
        Cli::Run(opts) => *opts,
        Cli::Help => panic!("expected Cli::Run, got Cli::Help"),
    }
}

#[test]
fn test_serialize_and_parse_cfg_payload_roundtrip() {
    let options = Options {
        template: PathBuf::from("tpl"),
        output: PathBuf::from("out"),
        stage2_bootstrap: "_main/pkg/stage2.py".to_string(),
        python_binary: "_main/pkg/.venv/bin/python3".to_string(),
        python_binary_actual: "python3_x86_64/bin/python3".to_string(),
        is_zipfile: true,
        recreate_venv_at_runtime: true,
        resolve_python_binary_at_runtime: false,
        venv_rel_site_packages: "lib/python3.11/site-packages".to_string(),
        workspace_name: "_main".to_string(),
        target: "//pkg:bin".to_string(),
        use_eof_trailer: false,
        interpreter_args: vec!["-B".to_string(), "-Xutf8".to_string()],
        runtime_venv_symlinks: vec![
            RuntimeVenvSymlink {
                venv_path: "lib/foo".to_string(),
                link_to_path: "other/lib/foo".to_string(),
            },
            RuntimeVenvSymlink {
                venv_path: "bin/helper".to_string(),
                link_to_path: "_main/pkg/helper".to_string(),
            },
        ],
    };

    let payload = serialize_cfg_payload(&options);
    let mut cfg = BootstrapConfig::default();
    parse_cfg_payload(&payload, &mut cfg);

    assert_eq!(cfg.stage2_bootstrap, "_main/pkg/stage2.py");
    assert_eq!(cfg.python_binary, "_main/pkg/.venv/bin/python3");
    assert_eq!(cfg.python_binary_actual, "python3_x86_64/bin/python3");
    assert_eq!(cfg.is_zipfile, 1);
    assert_eq!(cfg.recreate_venv_at_runtime, 1);
    assert_eq!(cfg.resolve_python_binary_at_runtime, 0);
    assert_eq!(cfg.venv_rel_site_packages, "lib/python3.11/site-packages");
    assert_eq!(cfg.workspace_name, "_main");
    assert_eq!(cfg.target, "//pkg:bin");
    assert_eq!(
        cfg.interpreter_args,
        vec!["-B".to_string(), "-Xutf8".to_string()]
    );
    assert_eq!(
        cfg.runtime_venv_symlinks,
        vec![
            "lib/foo|other/lib/foo".to_string(),
            "bin/helper|_main/pkg/helper".to_string()
        ]
    );
}

#[test]
fn test_embed_config_into_binary_inplace_preserves_size_and_rejects_overflow() {
    let tpl = make_mock_template_binary();
    let options = Options {
        stage2_bootstrap: "_main/app_stage2.py".to_string(),
        python_binary: "_main/.app.venv/bin/python3".to_string(),
        python_binary_actual: "py_toolchain/bin/python3".to_string(),
        workspace_name: "_main".to_string(),
        target: "//:app".to_string(),
        interpreter_args: vec!["-XSPECIAL=1".to_string()],
        ..Options::default()
    };

    let embedded = embed_config_into_binary(&tpl, &options).expect("in-place embed should succeed");
    assert_eq!(embedded.len(), tpl.len());

    let offset = find_magic_offset(&embedded).expect("magic sentinel should be present");
    assert_eq!(offset, 128);

    let mut len_bytes = [0u8; 4];
    len_bytes.copy_from_slice(&embedded[offset + 32..offset + 36]);
    let plen = u32::from_le_bytes(len_bytes) as usize;
    assert!(plen > 0 && plen < CFG_BUFFER_SIZE - CFG_HEADER_LEN);

    let payload = &embedded[offset + CFG_HEADER_LEN..offset + CFG_HEADER_LEN + plen];
    let mut cfg = BootstrapConfig::default();
    parse_cfg_payload(payload, &mut cfg);
    assert_eq!(cfg.stage2_bootstrap, "_main/app_stage2.py");
    assert_eq!(cfg.python_binary, "_main/.app.venv/bin/python3");
    assert_eq!(cfg.target, "//:app");
    assert_eq!(cfg.interpreter_args, vec!["-XSPECIAL=1".to_string()]);

    let too_large = Options {
        interpreter_args: vec!["X".repeat(5000)],
        ..options
    };
    assert!(embed_config_into_binary(&tpl, &too_large).is_err());
}

#[test]
fn test_embed_config_into_binary_windows_eof_trailer_roundtrip() {
    let tpl = make_mock_windows_template_binary();
    let long_arg = "X".repeat(2000);
    let many_args: Vec<String> = (0..40).map(|i| format!("-D{i}={long_arg}")).collect();
    let options = Options {
        stage2_bootstrap: "_main/windows_stage2.py".to_string(),
        python_binary: "_main/.windows.venv/Scripts/python.exe".to_string(),
        target: "//:windows_app".to_string(),
        use_eof_trailer: true,
        interpreter_args: many_args.clone(),
        ..Options::default()
    };

    let embedded =
        embed_config_into_binary(&tpl, &options).expect("EOF trailer embed should succeed");
    assert!(embedded.len() > tpl.len());
    assert_eq!(&embedded[..tpl.len()], &tpl[..]);

    let mut cfg = BootstrapConfig::default();
    parse_eof_trailer_from_bytes(&embedded, &mut cfg).expect("EOF trailer should parse");
    assert_eq!(cfg.stage2_bootstrap, "_main/windows_stage2.py");
    assert_eq!(cfg.target, "//:windows_app");
    assert_eq!(cfg.interpreter_args, many_args);

    let tmp = TempDir::new("eof_file");
    let exe_path = tmp.path().join("windows_configured.exe");
    fs::write(&exe_path, &embedded).expect("exe should be writable");
    let loaded = load_eof_trailer_config(exe_path.to_str().expect("path should be UTF-8"))
        .expect("EOF trailer on disk should load");
    assert_eq!(loaded.stage2_bootstrap, "_main/windows_stage2.py");
    assert_eq!(loaded.interpreter_args.len(), 40);
}

#[test]
fn test_embed_bootstrap_file_and_missing_magic_error() {
    let tmp = TempDir::new("embed_file");
    let tpl_path = tmp.path().join("prebuilt_tpl.bin");
    let out_path = tmp.path().join("nested").join("out.bin");
    let tpl = make_mock_template_binary();
    fs::write(&tpl_path, &tpl).expect("template should be writable");

    let options = Options {
        template: tpl_path,
        output: out_path.clone(),
        stage2_bootstrap: "_main/s2.py".to_string(),
        python_binary: "_main/py3".to_string(),
        interpreter_args: vec!["-a".to_string(), "-b".to_string()],
        ..Options::default()
    };

    embed_bootstrap_file(&options).expect("embed_bootstrap_file should succeed");
    let embedded = fs::read(&out_path).expect("output should be readable");
    assert_eq!(embedded.len(), tpl.len());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&out_path)
            .expect("output metadata should be readable")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "output binary should be executable");
    }

    let text_tpl_path = tmp.path().join("custom_tpl.sh");
    fs::write(&text_tpl_path, b"#!/usr/bin/env bash\n").expect("text template should be writable");
    let bad_options = Options {
        template: text_tpl_path.clone(),
        ..options
    };
    let err = format!(
        "{:#}",
        embed_bootstrap_file(&bad_options).expect_err("non-native template should fail")
    );
    assert!(
        err.contains("missing native bootstrap magic sentinel"),
        "unexpected error: {err}"
    );
    assert!(
        err.contains(text_tpl_path.to_str().expect("path should be UTF-8")),
        "error should name template path: {err}"
    );
}

#[test]
fn test_parse_args_flagfile_canonical_labels_and_equals_syntax() {
    let tmp = TempDir::new("param_file");
    let param_file = tmp.path().join("args.params");
    fs::write(
        &param_file,
        "--template\nin.bin\r\n--output=out.bin\r\n--eof_trailer\n1\n--stage2_bootstrap\ns2.py\n--target\n@@//pkg:bin\n--runtime_venv_symlink\nLib/site-packages/pkg|+pypi+pkg/site-packages/pkg\n",
    )
    .expect("param file should be writable");

    let opts = unwrap_run_options(
        parse_args([
            format!("@{}", param_file.display()),
            "--is_zipfile=true".to_string(),
            "--interpreter_arg".to_string(),
            "@literal_arg".to_string(),
        ])
        .expect("param file and CLI args should parse"),
    );
    assert_eq!(opts.template, PathBuf::from("in.bin"));
    assert_eq!(opts.output, PathBuf::from("out.bin"));
    assert!(opts.use_eof_trailer);
    assert!(opts.is_zipfile);
    assert_eq!(opts.stage2_bootstrap, "s2.py");
    assert_eq!(opts.target, "@@//pkg:bin");
    assert_eq!(opts.interpreter_args, vec!["@literal_arg".to_string()]);
    assert_eq!(
        opts.runtime_venv_symlinks,
        vec![RuntimeVenvSymlink {
            venv_path: "Lib/site-packages/pkg".to_string(),
            link_to_path: "+pypi+pkg/site-packages/pkg".to_string(),
        }]
    );

    // Direct CLI `--target @@//pkg:bin` without a param file must also treat `@@//pkg:bin` as a
    // literal value rather than a response file.
    let cli_opts = unwrap_run_options(
        parse_args([
            "--template",
            "in.bin",
            "--output",
            "out.bin",
            "--target",
            "@@//pkg:bin",
        ])
        .expect("direct CLI --target @@//pkg:bin should parse"),
    );
    assert_eq!(cli_opts.target, "@@//pkg:bin");
}

#[test]
fn test_help_and_validation_errors() {
    assert_eq!(
        parse_args(["--help"]).expect("--help should parse"),
        Cli::Help
    );
    assert_eq!(parse_args(["-h"]).expect("-h should parse"), Cli::Help);
    assert_eq!(run_cli(["--help"]), ExitCode::SUCCESS);

    assert!(parse_args::<_, &str>([]).is_err());
    assert!(parse_args(["--template", "in.bin"]).is_err());
    assert!(parse_args(["--output", "out.bin"]).is_err());
    assert!(parse_args(["--template=a", "--output=b", "--unknown=1"]).is_err());
    assert!(parse_args(["--template=a", "--output=b", "extra_positional"]).is_err());
    assert!(parse_args(["@"]).is_err());
    assert!(parse_args(["--template=a", "--output=b", "--is_zipfile=maybe"]).is_err());
    assert!(
        parse_args([
            "--template=a",
            "--output=b",
            "--runtime_venv_symlink=no_pipe"
        ])
        .is_err()
    );
    assert!(parse_args(["--template=a", "--output=b", "--runtime_venv_symlink=|rhs"]).is_err());
    assert!(parse_args(["--template=a", "--output=b", "--target=bad\0nul"]).is_err());

    assert_eq!(run_cli::<_, &str>([]), ExitCode::FAILURE);
}

#[test]
fn test_shrubbery_cli_subprocess_in_cwd() {
    let shrubbery_bin = PathBuf::from(
        env::var_os("SHRUBBERY_BIN").expect("SHRUBBERY_BIN should be set by the test runner"),
    );
    let abs_shrubbery = std::path::absolute(&shrubbery_bin)
        .expect("SHRUBBERY_BIN should resolve to an absolute path");

    let tmp = TempDir::new("subprocess_cwd");
    let tpl = make_mock_template_binary();
    fs::write(tmp.path().join("tpl.bin"), &tpl).expect("tpl.bin should be writable");

    let out = Command::new(&abs_shrubbery)
        .current_dir(tmp.path())
        .args([
            "native_bootstrap_embedder",
            "--template=tpl.bin",
            "--output=out.bin",
            "--stage2_bootstrap=_main/pkg/stage2.py",
            "--target",
            "@@//pkg:app",
        ])
        .output()
        .expect("shrubbery native_bootstrap_embedder should execute");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let embedded = fs::read(tmp.path().join("out.bin")).expect("out.bin should exist in cwd");
    assert_eq!(embedded.len(), tpl.len());
    let offset = find_magic_offset(&embedded).expect("magic sentinel should be present");
    let mut len_bytes = [0u8; 4];
    len_bytes.copy_from_slice(&embedded[offset + 32..offset + 36]);
    let plen = u32::from_le_bytes(len_bytes) as usize;
    let mut cfg = BootstrapConfig::default();
    parse_cfg_payload(
        &embedded[offset + CFG_HEADER_LEN..offset + CFG_HEADER_LEN + plen],
        &mut cfg,
    );
    assert_eq!(cfg.stage2_bootstrap, "_main/pkg/stage2.py");
    assert_eq!(cfg.target, "@@//pkg:app");
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let base = env::var_os("TEST_TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);
        let path = base.join(format!(
            "native_bootstrap_embedder_test_{}_{}_{}",
            prefix,
            std::process::id(),
            id
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir should be creatable");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
