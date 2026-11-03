use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use build_data_writer_lib::{
    expand_args, parse_args, parse_env_and_args, run, write_build_data, BuildDataOptions,
};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);
static ENV_MUTEX: Mutex<()> = Mutex::new(());

fn create_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = env::temp_dir().join(format!(
        "build_data_writer_test_{}_{}_{}",
        prefix,
        std::process::id(),
        id
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_env_configuration_both_status_files() {
    let temp_dir = create_temp_dir("env_both");
    let output_path = temp_dir.join("build_data.txt");
    let version_file = temp_dir.join("volatile-status.txt");
    let info_file = temp_dir.join("stable-status.txt");

    fs::write(
        &version_file,
        "BUILD_TIMESTAMP 1700000000\nFORMATTED_DATE 2023 Nov 14 22 13 20 Tue\n",
    )
    .unwrap();
    fs::write(&info_file, "BUILD_HOST build-box\nBUILD_USER builder\n").unwrap();

    let mut env_map = HashMap::new();
    env_map.insert(
        "OUTPUT".to_string(),
        output_path.to_string_lossy().into_owned(),
    );
    env_map.insert(
        "TARGET".to_string(),
        "//tests/build_data:build_data_test".to_string(),
    );
    env_map.insert("STAMPED".to_string(), "TRUE".to_string());
    env_map.insert(
        "VERSION_FILE".to_string(),
        version_file.to_string_lossy().into_owned(),
    );
    env_map.insert(
        "INFO_FILE".to_string(),
        info_file.to_string_lossy().into_owned(),
    );

    let opts = parse_env_and_args(&[], |k| env_map.get(k).cloned()).unwrap();
    write_build_data(&opts).unwrap();

    let result = fs::read_to_string(&output_path).unwrap();
    let expected = concat!(
        "TARGET //tests/build_data:build_data_test\n",
        "STAMPED TRUE\n",
        "BUILD_TIMESTAMP 1700000000\n",
        "FORMATTED_DATE 2023 Nov 14 22 13 20 Tue\n",
        "BUILD_HOST build-box\n",
        "BUILD_USER builder\n",
    );
    assert_eq!(result, expected);
    assert!(result.contains("TARGET "));
    assert!(result.contains("STAMPED TRUE"));
    assert!(result.contains("BUILD_HOST "));
    assert!(result.contains("BUILD_USER "));
    assert!(result.contains("BUILD_TIMESTAMP "));
    assert!(result.contains("FORMATTED_DATE "));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_process_env_run_end_to_end() {
    let _guard = ENV_MUTEX.lock().unwrap();
    let temp_dir = create_temp_dir("proc_env");
    let output_path = temp_dir.join("out.build_data.txt");
    let version_file = temp_dir.join("volatile.txt");
    let info_file = temp_dir.join("stable.txt");

    fs::write(&version_file, "BUILD_TIMESTAMP 42\n").unwrap();
    fs::write(&info_file, "BUILD_USER alice\n").unwrap();

    env::set_var("OUTPUT", &output_path);
    env::set_var("TARGET", "//pkg:bin");
    env::set_var("STAMPED", "TRUE");
    env::set_var("VERSION_FILE", &version_file);
    env::set_var("INFO_FILE", &info_file);

    let run_res = run(&[]);

    env::remove_var("OUTPUT");
    env::remove_var("TARGET");
    env::remove_var("STAMPED");
    env::remove_var("VERSION_FILE");
    env::remove_var("INFO_FILE");

    run_res.unwrap();
    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        content,
        "TARGET //pkg:bin\nSTAMPED TRUE\nBUILD_TIMESTAMP 42\nBUILD_USER alice\n"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_flags_space_and_equals_syntax() {
    let temp_dir = create_temp_dir("cli_flags");
    let output_path = temp_dir.join("cli_out.txt");
    let version_file = temp_dir.join("version.txt");
    let info_file = temp_dir.join("info.txt");

    fs::write(&version_file, "BUILD_TIMESTAMP 999\n").unwrap();
    fs::write(&info_file, "BUILD_HOST localhost\n").unwrap();

    let args = vec![
        "--output".to_string(),
        output_path.to_string_lossy().into_owned(),
        "--target=//my:target".to_string(),
        "--stamped".to_string(),
        "FALSE".to_string(),
        format!("--version_file={}", version_file.to_string_lossy()),
        "--info_file".to_string(),
        info_file.to_string_lossy().into_owned(),
    ];

    let opts = parse_env_and_args(&args, |_| None).unwrap();
    write_build_data(&opts).unwrap();

    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        content,
        "TARGET //my:target\nSTAMPED FALSE\nBUILD_TIMESTAMP 999\nBUILD_HOST localhost\n"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_cli_flags_override_and_supplement_env() {
    let temp_dir = create_temp_dir("override");
    let env_output = temp_dir.join("env_out.txt");
    let cli_output = temp_dir.join("cli_out.txt");
    let version_file = temp_dir.join("version.txt");

    fs::write(&version_file, "VOLATILE_KEY val\n").unwrap();

    let mut env_map = HashMap::new();
    env_map.insert(
        "OUTPUT".to_string(),
        env_output.to_string_lossy().into_owned(),
    );
    env_map.insert("TARGET".to_string(), "//env:target".to_string());
    env_map.insert("STAMPED".to_string(), "FALSE".to_string());
    env_map.insert(
        "VERSION_FILE".to_string(),
        version_file.to_string_lossy().into_owned(),
    );

    let args = vec![
        "--output".to_string(),
        cli_output.to_string_lossy().into_owned(),
        "--stamped=TRUE".to_string(),
    ];

    let opts = parse_env_and_args(&args, |k| env_map.get(k).cloned()).unwrap();
    write_build_data(&opts).unwrap();

    assert!(!env_output.exists());
    let content = fs::read_to_string(&cli_output).unwrap();
    assert_eq!(
        content,
        "TARGET //env:target\nSTAMPED TRUE\nVOLATILE_KEY val\n"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_empty_or_unset_version_and_info_files() {
    let temp_dir = create_temp_dir("empty_unset");
    let output_path = temp_dir.join("unstamped.txt");
    let empty_file = temp_dir.join("empty.txt");
    fs::write(&empty_file, "").unwrap();

    // Case 1: Both VERSION_FILE and INFO_FILE unset
    let opts = BuildDataOptions {
        output: output_path.clone(),
        target: "//foo:bar".to_string(),
        stamped: "FALSE".to_string(),
        version_file: None,
        info_file: None,
    };
    write_build_data(&opts).unwrap();
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "TARGET //foo:bar\nSTAMPED FALSE\n"
    );

    // Case 2: VERSION_FILE and INFO_FILE set to empty strings in env
    let mut env_map = HashMap::new();
    env_map.insert(
        "OUTPUT".to_string(),
        output_path.to_string_lossy().into_owned(),
    );
    env_map.insert("TARGET".to_string(), "//foo:baz".to_string());
    env_map.insert("STAMPED".to_string(), "TRUE".to_string());
    env_map.insert("VERSION_FILE".to_string(), "".to_string());
    env_map.insert("INFO_FILE".to_string(), "".to_string());

    let parsed = parse_env_and_args(&[], |k| env_map.get(k).cloned()).unwrap();
    assert_eq!(parsed.version_file, None);
    assert_eq!(parsed.info_file, None);
    write_build_data(&parsed).unwrap();
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "TARGET //foo:baz\nSTAMPED TRUE\n"
    );

    // Case 3: VERSION_FILE points to a 0-byte file
    let opts_empty_file = BuildDataOptions {
        output: output_path.clone(),
        target: "//foo:qux".to_string(),
        stamped: "TRUE".to_string(),
        version_file: Some(empty_file.clone()),
        info_file: None,
    };
    write_build_data(&opts_empty_file).unwrap();
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "TARGET //foo:qux\nSTAMPED TRUE\n"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_status_files_without_trailing_newline() {
    let temp_dir = create_temp_dir("no_newline");
    let output_path = temp_dir.join("out.txt");
    let version_file = temp_dir.join("volatile.txt");
    let info_file = temp_dir.join("stable.txt");

    // Neither file ends with '\n'
    fs::write(&version_file, "BUILD_TIMESTAMP 123").unwrap();
    fs::write(&info_file, "BUILD_USER bob").unwrap();

    let opts = BuildDataOptions {
        output: output_path.clone(),
        target: "//a:b".to_string(),
        stamped: "TRUE".to_string(),
        version_file: Some(version_file),
        info_file: Some(info_file),
    };
    write_build_data(&opts).unwrap();

    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        content,
        "TARGET //a:b\nSTAMPED TRUE\nBUILD_TIMESTAMP 123\nBUILD_USER bob\n"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_truncates_existing_output_and_creates_parent_dirs() {
    let temp_dir = create_temp_dir("truncate_nested");
    let output_path = temp_dir.join("nested").join("dir").join("build_data.txt");

    let opts1 = BuildDataOptions {
        output: output_path.clone(),
        target: "//first:target_with_long_previous_content".to_string(),
        stamped: "TRUE".to_string(),
        version_file: None,
        info_file: None,
    };
    write_build_data(&opts1).unwrap();

    let opts2 = BuildDataOptions {
        output: output_path.clone(),
        target: "//second:short".to_string(),
        stamped: "FALSE".to_string(),
        version_file: None,
        info_file: None,
    };
    write_build_data(&opts2).unwrap();

    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(content, "TARGET //second:short\nSTAMPED FALSE\n");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_flagfile_expansion() {
    let temp_dir = create_temp_dir("flagfile");
    let output_path = temp_dir.join("out.txt");
    let flagfile = temp_dir.join("params.txt");

    fs::write(
        &flagfile,
        format!(
            "--output\n{}\n--target=//flag:file\n--stamped=TRUE\n",
            output_path.to_string_lossy()
        ),
    )
    .unwrap();

    let raw_args = vec![format!("@{}", flagfile.to_string_lossy())];
    let expanded = expand_args(&raw_args).unwrap();
    assert_eq!(expanded.len(), 4);

    let opts = parse_env_and_args(&raw_args, |_| None).unwrap();
    write_build_data(&opts).unwrap();

    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(content, "TARGET //flag:file\nSTAMPED TRUE\n");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_missing_output_error_handling() {
    let _guard = ENV_MUTEX.lock().unwrap();
    env::remove_var("OUTPUT");

    // Missing from both env and CLI
    let err = parse_args(&[]).unwrap_err();
    assert!(err.contains("OUTPUT"), "unexpected error: {}", err);

    // Explicitly empty --output="" overrides env and fails
    let mut env_map = HashMap::new();
    env_map.insert("OUTPUT".to_string(), "/tmp/some_path.txt".to_string());
    let err2 =
        parse_env_and_args(&["--output=".to_string()], |k| env_map.get(k).cloned()).unwrap_err();
    assert!(err2.contains("OUTPUT"), "unexpected error: {}", err2);

    // Direct call to write_build_data with empty output path fails
    let bad_opts = BuildDataOptions::default();
    assert!(write_build_data(&bad_opts).is_err());
}

#[test]
fn test_invalid_cli_args_and_missing_status_file_errors() {
    let temp_dir = create_temp_dir("errors");
    let output_path = temp_dir.join("out.txt");

    // Unknown flag
    let err_unknown = parse_env_and_args(&["--unknown-flag".to_string()], |_| None).unwrap_err();
    assert!(err_unknown.contains("unexpected argument"));

    // Flag missing value
    let err_missing_val = parse_env_and_args(&["--output".to_string()], |_| None).unwrap_err();
    assert!(err_missing_val.contains("requires a value"));

    // Nonexistent VERSION_FILE fails on write_build_data
    let opts = BuildDataOptions {
        output: output_path,
        target: "//t:t".to_string(),
        stamped: "TRUE".to_string(),
        version_file: Some(temp_dir.join("does_not_exist.txt")),
        info_file: None,
    };
    assert!(write_build_data(&opts).is_err());

    let _ = fs::remove_dir_all(&temp_dir);
}
