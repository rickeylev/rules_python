use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use uv_lock_lib::{
    ensure_parent_dir, make_writable, read_trailer, remove_file_safely, run, Config,
    EmbeddedPayload, MAGIC,
};

fn find_embed_args_sh() -> PathBuf {
    if let Ok(srcdir) = env::var("TEST_SRCDIR") {
        for workspace in [
            env::var("TEST_WORKSPACE").unwrap_or_default(),
            "rules_python".to_string(),
            "_main".to_string(),
        ] {
            if !workspace.is_empty() {
                let p = PathBuf::from(&srcdir)
                    .join(&workspace)
                    .join("tools/embed_args/embed_args.sh");
                if p.exists() {
                    return p;
                }
            }
        }
    }
    if let Ok(runfiles) = env::var("RUNFILES_DIR") {
        for workspace in ["rules_python", "_main"] {
            let p = PathBuf::from(&runfiles)
                .join(workspace)
                .join("tools/embed_args/embed_args.sh");
            if p.exists() {
                return p;
            }
        }
    }
    PathBuf::from("tools/embed_args/embed_args.sh")
}

#[test]
fn test_payload_from_bytes() {
    let raw = b"src_out=tests/uv/lock/testdata/uv_lock_expected.lock\0out=bazel-out/k8-fastbuild/bin/out.lock\0arg=uv\0arg=lock\0arg=--project\0arg=tests/uv/lock/testdata\0arg=--quotes \"hello\"\0arg=backslash \\ path\0";
    let parsed = EmbeddedPayload::from_bytes(raw).unwrap();
    let expected = EmbeddedPayload {
        src_out: Some("tests/uv/lock/testdata/uv_lock_expected.lock".to_string()),
        out: Some("bazel-out/k8-fastbuild/bin/out.lock".to_string()),
        args: vec![
            "uv".to_string(),
            "lock".to_string(),
            "--project".to_string(),
            "tests/uv/lock/testdata".to_string(),
            "--quotes \"hello\"".to_string(),
            "backslash \\ path".to_string(),
        ],
    };
    assert_eq!(expected, parsed);
}

#[test]
fn test_embed_and_read_trailer() {
    let temp_dir = env::temp_dir().join(format!("uv_lock_trailer_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let modified_binary = temp_dir.join("modified_binary.bin");

    let payload_bytes = b"src_out=my_project/uv.lock\0out=bazel-bin/my_project/uv.lock.out\0arg=uv\0arg=lock\0arg=--no-cache\0";

    let mut out_file = File::create(&modified_binary).unwrap();
    out_file.write_all(b"\x7fELFfakeexecutablebinarycontents").unwrap();
    let len_bytes = (payload_bytes.len() as u64).to_le_bytes();
    out_file.write_all(payload_bytes).unwrap();
    out_file.write_all(&len_bytes).unwrap();
    out_file.write_all(MAGIC).unwrap();
    out_file.flush().unwrap();

    let read_back = read_trailer(&modified_binary).unwrap().unwrap();
    let expected = EmbeddedPayload {
        src_out: Some("my_project/uv.lock".to_string()),
        out: Some("bazel-bin/my_project/uv.lock.out".to_string()),
        args: vec![
            "uv".to_string(),
            "lock".to_string(),
            "--no-cache".to_string(),
        ],
    };
    assert_eq!(expected, read_back);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_shell_embed_interop() {
    let temp_dir = env::temp_dir().join(format!("uv_lock_sh_interop_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let base_exe = temp_dir.join("base_dummy.bin");
    let output_exe = temp_dir.join("sh_configured.bin");

    fs::write(&base_exe, b"dummybinarybytes").unwrap();

    let sh_script = find_embed_args_sh();
    let status = Command::new("sh")
        .arg(&sh_script)
        .arg(&base_exe)
        .arg(&output_exe)
        .arg("src_out=workspace/uv.lock")
        .arg("out=bazel-out/uv.lock.out")
        .arg("arg=uv")
        .arg("arg=lock")
        .arg("arg=--project")
        .arg("arg=workspace")
        .status()
        .unwrap();

    assert!(status.success());
    assert!(output_exe.exists());

    let payload = read_trailer(&output_exe).unwrap().unwrap();
    assert_eq!(payload.src_out, Some("workspace/uv.lock".to_string()));
    assert_eq!(payload.out, Some("bazel-out/uv.lock.out".to_string()));
    assert_eq!(
        payload.args,
        vec![
            "uv".to_string(),
            "lock".to_string(),
            "--project".to_string(),
            "workspace".to_string(),
        ]
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_config_parsing_flags() {
    let mut config = Config::default();
    let args = vec![
        OsString::from("--src-out"),
        OsString::from("path/to/src_out.lock"),
        OsString::from("--out"),
        OsString::from("path/to/out.lock"),
        OsString::from("--"),
        OsString::from("uv"),
        OsString::from("lock"),
        OsString::from("--project"),
        OsString::from("foo"),
    ];

    config.merge_cli_args(args).unwrap();
    assert_eq!(
        config.src_out,
        Some(PathBuf::from("path/to/src_out.lock"))
    );
    assert_eq!(config.out, Some(PathBuf::from("path/to/out.lock")));
    assert_eq!(
        config.command,
        vec![
            OsString::from("uv"),
            OsString::from("lock"),
            OsString::from("--project"),
            OsString::from("foo")
        ]
    );
}

#[test]
fn test_config_parsing_inline_flags() {
    let mut config = Config::default();
    let args = vec![
        OsString::from("--src-out=my_src.lock"),
        OsString::from("--out=my_out.lock"),
        OsString::from("uv"),
        OsString::from("lock"),
    ];

    config.merge_cli_args(args).unwrap();
    assert_eq!(config.src_out, Some(PathBuf::from("my_src.lock")));
    assert_eq!(config.out, Some(PathBuf::from("my_out.lock")));
    assert_eq!(
        config.command,
        vec![OsString::from("uv"), OsString::from("lock")]
    );
}

#[test]
fn test_run_in_workspace() {
    let config = Config {
        src_out: None,
        out: None,
        #[cfg(windows)]
        command: vec![
            OsString::from("cmd"),
            OsString::from("/c"),
            OsString::from("exit 0"),
        ],
        #[cfg(not(windows))]
        command: vec![OsString::from("true")],
    };

    let exit_code = run(&config, true).unwrap();
    assert_eq!(exit_code, 0);
}

#[test]
fn test_run_build_action_src_out_exists() {
    let temp_dir = env::temp_dir().join(format!("uv_lock_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let src_out = temp_dir.join("src").join("uv.lock");
    let out = temp_dir.join("out").join("uv.lock");

    ensure_parent_dir(&src_out).unwrap();
    {
        let mut f = File::create(&src_out).unwrap();
        f.write_all(b"original content").unwrap();
    }

    #[cfg(windows)]
    let command = vec![
        OsString::from("cmd"),
        OsString::from("/c"),
        OsString::from(format!(
            "echo updated content > \"{}\"",
            src_out.display()
        )),
    ];
    #[cfg(not(windows))]
    let command = vec![
        OsString::from("sh"),
        OsString::from("-c"),
        OsString::from(format!(
            "echo 'updated content' > '{}'",
            src_out.display()
        )),
    ];

    let config = Config {
        src_out: Some(src_out.clone()),
        out: Some(out.clone()),
        command,
    };

    let exit_code = run(&config, false).unwrap();
    assert_eq!(exit_code, 0);
    assert!(out.exists());
    let out_content = fs::read_to_string(&out).unwrap();
    assert!(out_content.contains("updated content"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_run_build_action_src_out_does_not_exist() {
    let temp_dir = env::temp_dir().join(format!("uv_lock_test_no_src_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let src_out = temp_dir.join("src").join("uv.lock");
    let out = temp_dir.join("out").join("uv.lock");

    ensure_parent_dir(&src_out).unwrap();
    assert!(!src_out.exists());

    #[cfg(windows)]
    let command = vec![
        OsString::from("cmd"),
        OsString::from("/c"),
        OsString::from(format!(
            "echo generated content > \"{}\"",
            src_out.display()
        )),
    ];
    #[cfg(not(windows))]
    let command = vec![
        OsString::from("sh"),
        OsString::from("-c"),
        OsString::from(format!(
            "echo 'generated content' > '{}'",
            src_out.display()
        )),
    ];

    let config = Config {
        src_out: Some(src_out.clone()),
        out: Some(out.clone()),
        command,
    };

    let exit_code = run(&config, false).unwrap();
    assert_eq!(exit_code, 0);
    assert!(out.exists());
    let out_content = fs::read_to_string(&out).unwrap();
    assert!(out_content.contains("generated content"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_read_only_handling() {
    let temp_dir = env::temp_dir().join(format!("uv_lock_test_ro_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let test_file = temp_dir.join("readonly.txt");
    {
        let mut f = File::create(&test_file).unwrap();
        f.write_all(b"readonly content").unwrap();
    }

    let mut perms = fs::metadata(&test_file).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&test_file, perms).unwrap();

    make_writable(&test_file).unwrap();
    remove_file_safely(&test_file).unwrap();
    assert!(!test_file.exists());

    let _ = fs::remove_dir_all(&temp_dir);
}
