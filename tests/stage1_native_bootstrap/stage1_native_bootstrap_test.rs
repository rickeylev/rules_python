use std::env;
use std::fs;

use stage1_native_bootstrap_lib::{
    BootstrapConfig, CFG_BUFFER_SIZE, CFG_MAGIC, CFG_MAGIC_LEN, EOF_TRAILER_MAGIC, basename_str,
    create_temp_dir, dirname_str, extract_runfiles_prefix, find_magic_offset,
    find_runfiles_root_with_env, has_magic_sentinel, join_path, join_path3,
    load_eof_trailer_config, parse_cfg_payload, parse_eof_trailer_from_bytes,
    strip_last_two_components,
};

#[test]
fn test_find_magic_offset_skips_rodata_copy() {
    let mut bin = Vec::new();
    bin.extend_from_slice(&CFG_MAGIC);
    bin.resize(CFG_BUFFER_SIZE + 64, 0xCC);
    assert!(has_magic_sentinel(&bin));
    assert_eq!(find_magic_offset(&bin), None);

    let real_offset = bin.len();
    bin.resize(real_offset + CFG_BUFFER_SIZE, 0);
    bin[real_offset..real_offset + CFG_MAGIC_LEN].copy_from_slice(&CFG_MAGIC);
    bin.extend_from_slice(&[0xDD; 32]);

    assert_eq!(find_magic_offset(&bin), Some(real_offset));
}

#[test]
fn test_parse_cfg_payload_and_eof_trailer() {
    let payload = b"stage2_bootstrap=_main/s2.py\0is_zipfile=1\0interpreter_arg=-B\0\0";
    let mut cfg = BootstrapConfig::default();
    parse_cfg_payload(payload, &mut cfg);
    assert_eq!(cfg.stage2_bootstrap, "_main/s2.py");
    assert_eq!(cfg.is_zipfile, 1);
    assert_eq!(cfg.interpreter_args, vec!["-B".to_string()]);

    let temp_dir = env::var_os("TEST_TMPDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(format!("stage1_eof_trailer_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).expect("temp dir should be creatable");

    let mut exe_bytes = b"MZ".to_vec();
    exe_bytes.resize(64, 0xAA);
    exe_bytes.extend_from_slice(&CFG_MAGIC);
    let unconfigured_path = temp_dir.join("unconfigured.exe");
    fs::write(&unconfigured_path, &exe_bytes).expect("unconfigured exe should be writable");
    assert!(load_eof_trailer_config(&unconfigured_path.to_string_lossy()).is_err());

    exe_bytes.extend_from_slice(payload);
    exe_bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    exe_bytes.extend_from_slice(&EOF_TRAILER_MAGIC);

    let mut trailer_cfg = BootstrapConfig::default();
    parse_eof_trailer_from_bytes(&exe_bytes, &mut trailer_cfg)
        .expect("EOF trailer bytes should parse");
    assert_eq!(trailer_cfg.stage2_bootstrap, "_main/s2.py");

    let configured_path = temp_dir.join("configured.exe");
    fs::write(&configured_path, &exe_bytes).expect("configured exe should be writable");
    let loaded = load_eof_trailer_config(&configured_path.to_string_lossy())
        .expect("EOF trailer file should load");
    assert_eq!(loaded.stage2_bootstrap, "_main/s2.py");
    assert_eq!(loaded.interpreter_args, vec!["-B".to_string()]);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_path_helpers_and_extract_runfiles_prefix() {
    assert_eq!(join_path("/a/b", "c"), "/a/b/c");
    assert_eq!(join_path("/a/b/", "c"), "/a/b/c");
    assert_eq!(join_path("", "c"), "c");
    assert_eq!(join_path3("/a", "b", "c"), "/a/b/c");
    assert_eq!(dirname_str("/a/b/c"), "/a/b");
    assert_eq!(dirname_str("/a"), "/");
    assert_eq!(dirname_str("c"), ".");
    assert_eq!(basename_str("/a/b/python3"), "python3");
    assert_eq!(basename_str("python3"), "python3");
    assert_eq!(
        strip_last_two_components("_main/pkg/_bin.venv/bin/python3"),
        "_main/pkg/_bin.venv"
    );

    assert_eq!(
        extract_runfiles_prefix("/tmp/foo.runfiles/_main/pkg/bin"),
        Some("/tmp/foo.runfiles".to_string())
    );
    assert_eq!(
        extract_runfiles_prefix("/tmp/outer.runfiles/_main/inner.runfiles/_main/bin"),
        Some("/tmp/outer.runfiles/_main/inner.runfiles".to_string())
    );
    assert_eq!(extract_runfiles_prefix("/tmp/no_runfiles_here"), None);
}

#[test]
fn test_find_runfiles_root_with_env_and_symlinks() {
    let base_tmp = env::var_os("TEST_TMPDIR")
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| env::temp_dir().to_string_lossy().into_owned());
    let root_tmp = create_temp_dir(&base_tmp, "rf_test.").expect("temp dir should be creatable");

    let rf_dir = join_path(&root_tmp, "app.runfiles");
    let s2_rel = "_main/pkg/stage2.py";
    let s2_abs = join_path(&rf_dir, s2_rel);
    fs::create_dir_all(dirname_str(&s2_abs)).expect("stage2 dir should be creatable");
    fs::write(&s2_abs, b"# stage2").expect("stage2 file should be writable");

    // 1. Valid RUNFILES_DIR containing stage2_bootstrap
    let found =
        find_runfiles_root_with_env("/nonexistent/app", s2_rel, Some(&rf_dir), None, &root_tmp)
            .expect("valid RUNFILES_DIR should resolve");
    assert_eq!(found, rf_dir);

    // 2. Valid RUNFILES_MANIFEST_FILE ending in .runfiles_manifest
    let manifest_path = format!("{rf_dir}_manifest");
    let found_from_manifest = find_runfiles_root_with_env(
        "/nonexistent/app",
        s2_rel,
        None,
        Some(&manifest_path),
        &root_tmp,
    )
    .expect("valid RUNFILES_MANIFEST_FILE should resolve");
    assert_eq!(found_from_manifest, rf_dir);

    // 3. Invalid outer RUNFILES_DIR falls back to <argv0>.runfiles directory
    let wrong_rf = join_path(&root_tmp, "wrong.runfiles");
    fs::create_dir_all(&wrong_rf).expect("wrong.runfiles should be creatable");
    let app_bin = join_path(&root_tmp, "app");
    fs::write(&app_bin, b"").expect("app binary stub should be writable");
    let found_from_argv0 =
        find_runfiles_root_with_env(&app_bin, s2_rel, Some(&wrong_rf), None, &root_tmp)
            .expect("argv0.runfiles fallback should resolve");
    assert_eq!(found_from_argv0, rf_dir);

    // 4. Symlink hop to binary inside *.runfiles/*
    #[cfg(unix)]
    {
        let target_in_rf = join_path(&rf_dir, "_main/pkg/app_real");
        fs::write(&target_in_rf, b"").expect("target_in_rf should be writable");
        let link_path = join_path(&root_tmp, "app_link");
        std::os::unix::fs::symlink(&target_in_rf, &link_path).expect("symlink should be creatable");
        let found_via_link = find_runfiles_root_with_env(&link_path, s2_rel, None, None, &root_tmp)
            .expect("symlink hop should resolve runfiles");
        assert_eq!(found_via_link, rf_dir);
    }

    let _ = fs::remove_dir_all(&root_tmp);
}
