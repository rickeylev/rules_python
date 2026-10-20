use std::env;
use std::fs;

use stage1_native_bootstrap_lib::{
    basename_str, create_temp_dir, dirname_str, embed_bootstrap_file,
    embed_config_into_binary, extract_runfiles_prefix, find_magic_offset,
    find_runfiles_root_with_env, join_path, join_path3, load_eof_trailer_config,
    parse_cfg_payload, parse_embed_args, parse_eof_trailer_from_bytes,
    serialize_cfg_payload, strip_last_two_components, BootstrapConfig,
    EmbedOptions, CFG_BUFFER_SIZE, CFG_HEADER_LEN, CFG_MAGIC, CFG_MAGIC_LEN,
};

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

#[test]
fn test_serialize_and_parse_cfg_payload_roundtrip() {
    let opts = EmbedOptions {
        template_path: "tpl".to_string(),
        output_path: "out".to_string(),
        stage2_bootstrap: "_main/pkg/stage2.py".to_string(),
        python_binary: "_main/pkg/.venv/bin/python3".to_string(),
        python_binary_actual: "python3_x86_64/bin/python3".to_string(),
        is_zipfile: "1".to_string(),
        recreate_venv_at_runtime: "1".to_string(),
        resolve_python_binary_at_runtime: "0".to_string(),
        venv_rel_site_packages: "lib/python3.11/site-packages".to_string(),
        workspace_name: "_main".to_string(),
        target: "//pkg:bin".to_string(),
        use_eof_trailer: false,
        interpreter_args: vec!["-B".to_string(), "-Xutf8".to_string()],
        runtime_venv_symlinks: vec![
            "lib/foo|other/lib/foo".to_string(),
            "bin/helper|_main/pkg/helper".to_string(),
        ],
    };

    let payload = serialize_cfg_payload(&opts);
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
fn test_embed_config_into_binary_inplace_preserves_size() {
    let tpl = make_mock_template_binary();
    let opts = EmbedOptions {
        stage2_bootstrap: "_main/app_stage2.py".to_string(),
        python_binary: "_main/.app.venv/bin/python3".to_string(),
        python_binary_actual: "py_toolchain/bin/python3".to_string(),
        workspace_name: "_main".to_string(),
        target: "//:app".to_string(),
        interpreter_args: vec!["-XSPECIAL=1".to_string()],
        ..EmbedOptions::default()
    };

    let embedded = embed_config_into_binary(&tpl, &opts).unwrap();
    assert_eq!(embedded.len(), tpl.len());

    let offset = find_magic_offset(&embedded).expect("magic should be present");
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

    let too_large_opts = EmbedOptions {
        interpreter_args: vec!["X".repeat(5000)],
        ..opts
    };
    assert!(embed_config_into_binary(&tpl, &too_large_opts).is_err());
}

#[test]
fn test_find_magic_offset_skips_rodata_copy() {
    let mut bin = Vec::new();
    bin.extend_from_slice(&CFG_MAGIC);
    bin.resize(CFG_BUFFER_SIZE + 64, 0xCC);
    let real_offset = bin.len();
    bin.resize(real_offset + CFG_BUFFER_SIZE, 0);
    bin[real_offset..real_offset + CFG_MAGIC_LEN].copy_from_slice(&CFG_MAGIC);
    bin.extend_from_slice(&[0xDD; 32]);

    assert_eq!(find_magic_offset(&bin), Some(real_offset));
}

#[test]
fn test_embed_config_into_binary_windows_eof_trailer_roundtrip() {
    let tpl = make_mock_windows_template_binary();
    let long_arg = "X".repeat(2000);
    let mut many_args = Vec::new();
    for i in 0..40 {
        many_args.push(format!("-D{i}={long_arg}"));
    }
    let opts = EmbedOptions {
        stage2_bootstrap: "_main/windows_stage2.py".to_string(),
        python_binary: "_main/.windows.venv/Scripts/python.exe".to_string(),
        target: "//:windows_app".to_string(),
        use_eof_trailer: true,
        interpreter_args: many_args.clone(),
        ..EmbedOptions::default()
    };

    let embedded = embed_config_into_binary(&tpl, &opts).unwrap();
    assert!(embedded.len() > tpl.len());
    assert_eq!(&embedded[..tpl.len()], &tpl[..]);

    let mut cfg = BootstrapConfig::default();
    parse_eof_trailer_from_bytes(&embedded, &mut cfg).unwrap();
    assert_eq!(cfg.stage2_bootstrap, "_main/windows_stage2.py");
    assert_eq!(cfg.target, "//:windows_app");
    assert_eq!(cfg.interpreter_args, many_args);
}

#[test]
fn test_embed_bootstrap_file_and_missing_magic_error() {
    let temp_dir = env::temp_dir().join(format!(
        "stage1_native_bootstrap_test_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let tpl_path = temp_dir.join("prebuilt_tpl.bin");
    let out_path = temp_dir.join("nested").join("out.bin");
    let tpl = make_mock_template_binary();
    fs::write(&tpl_path, &tpl).unwrap();

    let opts = EmbedOptions {
        template_path: tpl_path.to_string_lossy().into_owned(),
        output_path: out_path.to_string_lossy().into_owned(),
        stage2_bootstrap: "_main/s2.py".to_string(),
        python_binary: "_main/py3".to_string(),
        interpreter_args: vec!["-a".to_string(), "-b".to_string()],
        ..EmbedOptions::default()
    };

    embed_bootstrap_file(&opts).unwrap();
    let embedded = fs::read(&out_path).unwrap();
    assert_eq!(embedded.len(), tpl.len());

    let text_tpl_path = temp_dir.join("custom_tpl.sh");
    fs::write(&text_tpl_path, b"#!/usr/bin/env bash\n").unwrap();
    let bad_opts = EmbedOptions {
        template_path: text_tpl_path.to_string_lossy().into_owned(),
        ..opts
    };
    assert!(embed_bootstrap_file(&bad_opts).is_err());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_load_eof_trailer_config_from_file() {
    let temp_dir = env::temp_dir().join(format!(
        "stage1_eof_trailer_file_test_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();

    let tpl = make_mock_windows_template_binary();
    let unconfigured_path = temp_dir.join("unconfigured.exe");
    fs::write(&unconfigured_path, &tpl).unwrap();
    assert!(load_eof_trailer_config(&unconfigured_path.to_string_lossy()).is_err());

    let opts = EmbedOptions {
        stage2_bootstrap: "_main/file_eof.py".to_string(),
        interpreter_args: vec!["Y".repeat(5000)],
        ..EmbedOptions::default()
    };
    let embedded = embed_config_into_binary(&tpl, &opts).unwrap();
    let exe_path = temp_dir.join("windows_configured.exe");
    fs::write(&exe_path, &embedded).unwrap();

    let cfg = load_eof_trailer_config(&exe_path.to_string_lossy()).unwrap();
    assert_eq!(cfg.stage2_bootstrap, "_main/file_eof.py");
    assert_eq!(cfg.interpreter_args.len(), 1);
    assert_eq!(cfg.interpreter_args[0].len(), 5000);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_parse_embed_args_validation() {
    let temp_dir = env::temp_dir().join(format!(
        "stage1_param_file_test_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir).unwrap();
    let param_file = temp_dir.join("args.params");
    fs::write(
        &param_file,
        "--template\nin.bin\n--output\nout.bin\n--eof_trailer\n1\n--stage2_bootstrap\ns2.py\n--target\n@@//pkg:bin\n--runtime_venv_symlink\nLib/site-packages/pkg|+pypi+pkg/site-packages/pkg\n",
    )
    .unwrap();
    let param_args = vec![
        "embedder".to_string(),
        format!("@{}", param_file.to_string_lossy()),
    ];
    let param_opts = parse_embed_args(&param_args).unwrap();
    assert_eq!(param_opts.template_path, "in.bin");
    assert_eq!(param_opts.output_path, "out.bin");
    assert!(param_opts.use_eof_trailer);
    assert_eq!(param_opts.stage2_bootstrap, "s2.py");
    assert_eq!(param_opts.target, "@@//pkg:bin");
    assert_eq!(
        param_opts.runtime_venv_symlinks,
        vec!["Lib/site-packages/pkg|+pypi+pkg/site-packages/pkg".to_string()]
    );
    let _ = fs::remove_dir_all(&temp_dir);
    let args = vec![
        "embedder".to_string(),
        "--template".to_string(),
        "in.bin".to_string(),
        "--output".to_string(),
        "out.bin".to_string(),
        "--stage2_bootstrap".to_string(),
        "s2.py".to_string(),
        "--target".to_string(),
        "@@//pkg:bin".to_string(),
        "--interpreter_arg".to_string(),
        "-I".to_string(),
    ];
    let opts = parse_embed_args(&args).unwrap();
    assert_eq!(opts.template_path, "in.bin");
    assert_eq!(opts.output_path, "out.bin");
    assert_eq!(opts.stage2_bootstrap, "s2.py");
    assert_eq!(opts.target, "@@//pkg:bin");
    assert_eq!(opts.interpreter_args, vec!["-I".to_string()]);

    let missing_required = vec![
        "embedder".to_string(),
        "--template".to_string(),
        "in.bin".to_string(),
    ];
    assert!(parse_embed_args(&missing_required).is_err());

    let unknown_flag = vec![
        "embedder".to_string(),
        "--unknown".to_string(),
        "val".to_string(),
    ];
    assert!(parse_embed_args(&unknown_flag).is_err());

    let incomplete_flag = vec!["embedder".to_string(), "--template".to_string()];
    assert!(parse_embed_args(&incomplete_flag).is_err());
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
        extract_runfiles_prefix(
            "/tmp/outer.runfiles/_main/inner.runfiles/_main/bin"
        ),
        Some("/tmp/outer.runfiles/_main/inner.runfiles".to_string())
    );
    assert_eq!(extract_runfiles_prefix("/tmp/no_runfiles_here"), None);
}

#[test]
fn test_find_runfiles_root_with_env_and_symlinks() {
    let base_tmp = env::temp_dir().to_string_lossy().into_owned();
    let root_tmp = create_temp_dir(&base_tmp, "rf_test.").unwrap();

    let rf_dir = join_path(&root_tmp, "app.runfiles");
    let s2_rel = "_main/pkg/stage2.py";
    let s2_abs = join_path(&rf_dir, s2_rel);
    fs::create_dir_all(dirname_str(&s2_abs)).unwrap();
    fs::write(&s2_abs, b"# stage2").unwrap();

    // 1. Valid RUNFILES_DIR containing stage2_bootstrap
    let found = find_runfiles_root_with_env(
        "/nonexistent/app",
        s2_rel,
        Some(&rf_dir),
        None,
        &root_tmp,
    )
    .unwrap();
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
    .unwrap();
    assert_eq!(found_from_manifest, rf_dir);

    // 3. Invalid outer RUNFILES_DIR falls back to <argv0>.runfiles directory
    let wrong_rf = join_path(&root_tmp, "wrong.runfiles");
    fs::create_dir_all(&wrong_rf).unwrap();
    let app_bin = join_path(&root_tmp, "app");
    fs::write(&app_bin, b"").unwrap();
    let found_from_argv0 = find_runfiles_root_with_env(
        &app_bin,
        s2_rel,
        Some(&wrong_rf),
        None,
        &root_tmp,
    )
    .unwrap();
    assert_eq!(found_from_argv0, rf_dir);

    // 4. Symlink hop to binary inside *.runfiles/*
    #[cfg(unix)]
    {
        let target_in_rf = join_path(&rf_dir, "_main/pkg/app_real");
        fs::write(&target_in_rf, b"").unwrap();
        let link_path = join_path(&root_tmp, "app_link");
        std::os::unix::fs::symlink(&target_in_rf, &link_path).unwrap();
        let found_via_link = find_runfiles_root_with_env(
            &link_path,
            s2_rel,
            None,
            None,
            &root_tmp,
        )
        .unwrap();
        assert_eq!(found_via_link, rf_dir);
    }

    let _ = fs::remove_dir_all(&root_tmp);
}
