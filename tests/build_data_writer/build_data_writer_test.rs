use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use build_data_writer_lib::{
    BuildDataOptions, expand_args, parse_env_and_args, run_cli, write_build_data,
};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

const NO_ARGS: [&str; 0] = [];

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

    let env_map: HashMap<&str, OsString> = HashMap::from([
        ("OUTPUT", output_path.as_os_str().to_owned()),
        (
            "TARGET",
            OsString::from("//tests/build_data:build_data_test"),
        ),
        ("STAMPED", OsString::from("TRUE")),
        ("VERSION_FILE", version_file.as_os_str().to_owned()),
        ("INFO_FILE", info_file.as_os_str().to_owned()),
    ]);

    let opts = parse_env_and_args(NO_ARGS, |k| env_map.get(k).cloned()).unwrap();
    assert!(opts.stamped);
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

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_run_cli_end_to_end() {
    let temp_dir = create_temp_dir("run_cli");
    let output_path = temp_dir.join("out.build_data.txt");
    let version_file = temp_dir.join("volatile.txt");
    let info_file = temp_dir.join("stable.txt");

    fs::write(&version_file, "BUILD_TIMESTAMP 42\n").unwrap();
    fs::write(&info_file, "BUILD_USER alice\n").unwrap();

    let args = [
        OsString::from("--output"),
        output_path.as_os_str().to_owned(),
        OsString::from("--target"),
        OsString::from("//pkg:bin"),
        OsString::from("--stamped"),
        OsString::from("TRUE"),
        OsString::from("--version_file"),
        version_file.as_os_str().to_owned(),
        OsString::from("--info_file"),
        info_file.as_os_str().to_owned(),
    ];

    assert_eq!(run_cli(args), ExitCode::SUCCESS);
    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        content,
        "TARGET //pkg:bin\nSTAMPED TRUE\nBUILD_TIMESTAMP 42\nBUILD_USER alice\n"
    );

    assert_eq!(run_cli(["--unknown-flag"]), ExitCode::FAILURE);

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
        OsString::from("--output"),
        output_path.as_os_str().to_owned(),
        OsString::from("--target=//my:target"),
        OsString::from("--stamped"),
        OsString::from("FALSE"),
        OsString::from(format!("--version_file={}", version_file.display())),
        OsString::from("--info_file"),
        info_file.as_os_str().to_owned(),
    ];

    let opts = parse_env_and_args(args, |_| None).unwrap();
    assert!(!opts.stamped);
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

    let env_map: HashMap<&str, OsString> = HashMap::from([
        ("OUTPUT", env_output.as_os_str().to_owned()),
        ("TARGET", OsString::from("//env:target")),
        ("STAMPED", OsString::from("FALSE")),
        ("VERSION_FILE", version_file.as_os_str().to_owned()),
    ]);

    let args = vec![
        OsString::from("--output"),
        cli_output.as_os_str().to_owned(),
        OsString::from("--stamped=TRUE"),
    ];

    let opts = parse_env_and_args(args, |k| env_map.get(k).cloned()).unwrap();
    assert!(opts.stamped);
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
fn test_stamped_boolean_parsing_variants() {
    for truthy in ["TRUE", "true", "1"] {
        let opts =
            parse_env_and_args(["--output=/tmp/out.txt", "--stamped", truthy], |_| None).unwrap();
        assert!(opts.stamped, "expected true for {truthy}");
    }

    for falsy in ["FALSE", "false", "0", ""] {
        let opts =
            parse_env_and_args(["--output=/tmp/out.txt", "--stamped", falsy], |_| None).unwrap();
        assert!(!opts.stamped, "expected false for {falsy}");
    }

    let err =
        parse_env_and_args(["--output=/tmp/out.txt", "--stamped=maybe"], |_| None).unwrap_err();
    assert!(err.contains("STAMPED/--stamped"), "unexpected error: {err}");
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
        stamped: false,
        version_file: None,
        info_file: None,
    };
    write_build_data(&opts).unwrap();
    assert_eq!(
        fs::read_to_string(&output_path).unwrap(),
        "TARGET //foo:bar\nSTAMPED FALSE\n"
    );

    // Case 2: VERSION_FILE and INFO_FILE set to empty strings in env
    let env_map: HashMap<&str, OsString> = HashMap::from([
        ("OUTPUT", output_path.as_os_str().to_owned()),
        ("TARGET", OsString::from("//foo:baz")),
        ("STAMPED", OsString::from("TRUE")),
        ("VERSION_FILE", OsString::new()),
        ("INFO_FILE", OsString::new()),
    ]);

    let parsed = parse_env_and_args(NO_ARGS, |k| env_map.get(k).cloned()).unwrap();
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
        stamped: true,
        version_file: Some(empty_file),
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
        stamped: true,
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
        stamped: true,
        version_file: None,
        info_file: None,
    };
    write_build_data(&opts1).unwrap();

    let opts2 = BuildDataOptions {
        output: output_path.clone(),
        target: "//second:short".to_string(),
        stamped: false,
        version_file: None,
        info_file: None,
    };
    write_build_data(&opts2).unwrap();

    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(content, "TARGET //second:short\nSTAMPED FALSE\n");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_flagfile_expansion_and_blank_lines_and_bare_at() {
    let temp_dir = create_temp_dir("flagfile");
    let output_path = temp_dir.join("out.txt");
    let flagfile = temp_dir.join("params.txt");

    fs::write(
        &flagfile,
        format!(
            "--output\n{}\n\n--target=//flag:file\n--stamped=TRUE\n\n",
            output_path.display()
        ),
    )
    .unwrap();

    let raw_args = vec![format!("@{}", flagfile.display())];
    let expanded = expand_args(&raw_args).unwrap();
    assert_eq!(expanded.len(), 4);

    let opts = parse_env_and_args(&raw_args, |_| None).unwrap();
    write_build_data(&opts).unwrap();

    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(content, "TARGET //flag:file\nSTAMPED TRUE\n");

    assert!(expand_args(["@"]).is_err());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_missing_output_error_handling() {
    // Missing from both env and CLI
    let err = parse_env_and_args(NO_ARGS, |_| None).unwrap_err();
    assert!(err.contains("OUTPUT"), "unexpected error: {err}");

    // Empty OUTPUT env var also fails
    let err_empty_env =
        parse_env_and_args(NO_ARGS, |k| (k == "OUTPUT").then(OsString::new)).unwrap_err();
    assert!(
        err_empty_env.contains("OUTPUT"),
        "unexpected error: {err_empty_env}"
    );

    // Explicitly empty --output="" overrides env and fails
    let env_map: HashMap<&str, OsString> =
        HashMap::from([("OUTPUT", OsString::from("/tmp/some_path.txt"))]);
    let err2 = parse_env_and_args(["--output="], |k| env_map.get(k).cloned()).unwrap_err();
    assert!(err2.contains("OUTPUT"), "unexpected error: {err2}");
}

#[test]
fn test_invalid_cli_args_and_missing_status_file_errors() {
    let temp_dir = create_temp_dir("errors");
    let output_path = temp_dir.join("out.txt");

    // Unknown flag
    let err_unknown = parse_env_and_args(["--unknown-flag"], |_| None).unwrap_err();
    assert!(err_unknown.contains("unexpected argument"));

    // Flag missing value
    let err_missing_val = parse_env_and_args(["--output"], |_| None).unwrap_err();
    assert!(err_missing_val.contains("requires a value"));

    // Nonexistent VERSION_FILE fails on write_build_data
    let opts = BuildDataOptions {
        output: output_path,
        target: "//t:t".to_string(),
        stamped: true,
        version_file: Some(temp_dir.join("does_not_exist.txt")),
        info_file: None,
    };
    assert!(write_build_data(&opts).is_err());

    let _ = fs::remove_dir_all(&temp_dir);
}
