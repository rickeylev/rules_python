use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use zip_main_maker_lib::{
    build_substitutions, compute_inputs_hash, expand_template, generate_zip_main, parse_args,
    run_cli,
};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = env::temp_dir().join(format!(
            "zip_main_maker_test_{}_{}_{}",
            prefix,
            std::process::id(),
            id
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
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

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn test_creates_zip_main() {
    let tmp = TempDir::new("creates_zip_main");
    let template_path = tmp.path().join("template.py");
    fs::write(&template_path, "hash=%APP_HASH%\nfoo=%FOO%\n").unwrap();

    let output_path = tmp.path().join("output.py");

    let file1_path = tmp.path().join("file1.txt");
    fs::write(&file1_path, b"content1").unwrap();

    let file2_path = tmp.path().join("file2.txt");
    fs::write(&file2_path, b"content2").unwrap();

    let manifest_path = tmp.path().join("manifest.txt");

    #[cfg(unix)]
    let symlink_path = {
        let sp = tmp.path().join("symlink.txt");
        create_symlink(&file1_path, &sp);
        sp
    };

    let line1 = format!("rf-file|0|file1.txt|{}", file1_path.display());
    let line2 = format!("rf-file|0|file2.txt|{}", file2_path.display());
    #[cfg(unix)]
    let line3 = format!("rf-symlink|1|symlink.txt|{}", symlink_path.display());
    let line4 = "rf-empty|empty_file.txt".to_string();

    #[cfg(unix)]
    let raw_lines = vec![line1.clone(), line2.clone(), line3.clone(), line4.clone()];
    #[cfg(not(unix))]
    let raw_lines = vec![line1.clone(), line2.clone(), line4.clone()];

    fs::write(&manifest_path, format!("{}\n", raw_lines.join("\n"))).unwrap();

    let args: Vec<OsString> = vec![
        "--template".into(),
        template_path.as_os_str().to_owned(),
        "--output".into(),
        output_path.as_os_str().to_owned(),
        "--substitution".into(),
        "%FOO%=bar".into(),
        "--hash_files_manifest".into(),
        manifest_path.as_os_str().to_owned(),
    ];

    let options = parse_args(&args).unwrap();
    generate_zip_main(&options).unwrap();

    // Calculate expected hash matching Python reference test
    let mut sorted_lines = raw_lines.clone();
    sorted_lines.sort();

    let mut hasher = Sha256::new();
    for line in &sorted_lines {
        let (type_, rest) = line.split_once('|').unwrap_or((line, ""));
        hasher.update(rest.as_bytes());
        if type_ == "rf-empty" || type_ == "symlink" {
            continue;
        }
        let parts: Vec<&str> = rest.split('|').collect();
        let is_symlink_str = parts[0];
        let path = parts[parts.len() - 1];
        if is_symlink_str == "1" {
            let target = fs::read_link(path).unwrap();
            hasher.update(target.to_string_lossy().as_bytes());
        } else {
            hasher.update(fs::read(path).unwrap());
        }
    }
    let expected_hash = hex_encode(&hasher.finalize());

    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(content, format!("hash={}\nfoo=bar\n", expected_hash));
}

#[test]
fn test_unsorted_manifest_determinism() {
    let tmp = TempDir::new("unsorted_manifest");
    let f1 = tmp.path().join("a.txt");
    let f2 = tmp.path().join("b.txt");
    fs::write(&f1, b"alpha").unwrap();
    fs::write(&f2, b"beta").unwrap();

    let l1 = format!("rf-file|0|a.txt|{}", f1.display());
    let l2 = format!("rf-file|0|b.txt|{}", f2.display());
    let l3 = "rf-empty|z_empty.txt".to_string();
    let l4 = "symlink|runfiles_link|/some/target".to_string();

    let manifest_a = tmp.path().join("manifest_a.txt");
    let manifest_b = tmp.path().join("manifest_b.txt");

    fs::write(&manifest_a, format!("{}\n{}\n{}\n{}\n", l4, l2, l3, l1)).unwrap();
    fs::write(&manifest_b, format!("{}\n{}\n{}\n{}\n", l1, l2, l3, l4)).unwrap();

    let hash_a = compute_inputs_hash(&manifest_a).unwrap();
    let hash_b = compute_inputs_hash(&manifest_b).unwrap();
    assert_eq!(hash_a, hash_b);
    assert_eq!(hash_a.len(), 64);
}

#[test]
fn test_symlink_type_hashes_rest_only() {
    let tmp = TempDir::new("symlink_type");
    // A `symlink|<runfiles_path>|<link_to_path>` entry does not open `<link_to_path>` on disk;
    // it only hashes `rest` (`<runfiles_path>|<link_to_path>`).
    let manifest = tmp.path().join("manifest.txt");
    fs::write(
        &manifest,
        "symlink|pkg/link.py|/nonexistent/target/on/disk.py\nrf-empty|pkg/__init__.py\n",
    )
    .unwrap();

    let actual_hash = compute_inputs_hash(&manifest).unwrap();

    let mut hasher = Sha256::new();
    // "rf-empty|pkg/__init__.py" sorts before "symlink|pkg/link.py|/nonexistent/target/on/disk.py"
    hasher.update(b"pkg/__init__.py");
    hasher.update(b"pkg/link.py|/nonexistent/target/on/disk.py");
    let expected_hash = hex_encode(&hasher.finalize());

    assert_eq!(actual_hash, expected_hash);
}

#[cfg(unix)]
#[test]
fn test_is_symlink_minus_one_valid_vs_broken() {
    let tmp = TempDir::new("symlink_minus_one");
    let real_file = tmp.path().join("real.txt");
    fs::write(&real_file, b"real_file_bytes").unwrap();

    // Symlink pointing to an existing file: `os.path.exists(path)` is True -> `is_symlink` is False -> hashes file content!
    let valid_symlink = tmp.path().join("valid_sym.txt");
    create_symlink(&real_file, &valid_symlink);

    // Broken symlink pointing to a non-existent file: `os.path.exists(path)` is False -> `is_symlink` is True -> hashes readlink!
    let missing_target = tmp.path().join("does_not_exist.txt");
    let broken_symlink = tmp.path().join("broken_sym.txt");
    create_symlink(&missing_target, &broken_symlink);

    let manifest_valid = tmp.path().join("manifest_valid.txt");
    let line_valid = format!("rf-file|-1|valid_sym.txt|{}", valid_symlink.display());
    fs::write(&manifest_valid, format!("{}\n", line_valid)).unwrap();

    let hash_valid = compute_inputs_hash(&manifest_valid).unwrap();
    let mut h_valid = Sha256::new();
    h_valid.update(line_valid.split_once('|').unwrap().1.as_bytes());
    h_valid.update(b"real_file_bytes");
    assert_eq!(hash_valid, hex_encode(&h_valid.finalize()));

    let manifest_broken = tmp.path().join("manifest_broken.txt");
    let line_broken = format!("rf-file|-1|broken_sym.txt|{}", broken_symlink.display());
    fs::write(&manifest_broken, format!("{}\n", line_broken)).unwrap();

    let hash_broken = compute_inputs_hash(&manifest_broken).unwrap();
    let mut h_broken = Sha256::new();
    h_broken.update(line_broken.split_once('|').unwrap().1.as_bytes());
    h_broken.update(missing_target.to_string_lossy().as_bytes());
    assert_eq!(hash_broken, hex_encode(&h_broken.finalize()));
}

#[test]
fn test_flagfile_and_equals_flags_and_multiple_substitutions() {
    let tmp = TempDir::new("flagfile_and_subst");
    let template_path = tmp.path().join("template.py");
    fs::write(
        &template_path,
        "hash=%APP_HASH%\ndir=%EXTRACT_DIR%\nbin=%python_binary%\nws=%workspace_name%\n",
    )
    .unwrap();

    let output_path = tmp.path().join("nested").join("output.py");
    let manifest_path = tmp.path().join("manifest.txt");
    fs::write(&manifest_path, "rf-empty|__init__.py\n").unwrap();

    let flagfile_path = tmp.path().join("args.params");
    fs::write(
        &flagfile_path,
        format!(
            "--template={}\n--output\n{}\n--substitution=%EXTRACT_DIR%=_main/pkg/app\n--substitution=%python_binary%=old_val\n",
            template_path.display(),
            output_path.display(),
        ),
    )
    .unwrap();

    let args = vec![
        format!("@{}", flagfile_path.display()),
        "--substitution".to_string(),
        "%python_binary%=venv/bin/python3".to_string(),
        "--substitution=%workspace_name%=my_ws=with_eq".to_string(),
        format!("--hash_files_manifest={}", manifest_path.display()),
    ];

    let options = parse_args(&args).unwrap();
    assert_eq!(options.template, template_path);
    assert_eq!(options.output, output_path);
    assert_eq!(options.hash_files_manifest, manifest_path);
    assert_eq!(run_cli(&args), ExitCode::SUCCESS);

    let expected_hash = compute_inputs_hash(&manifest_path).unwrap();
    let content = fs::read_to_string(&output_path).unwrap();
    assert_eq!(
        content,
        format!(
            "hash={}\ndir=_main/pkg/app\nbin=venv/bin/python3\nws=my_ws=with_eq\n",
            expected_hash
        )
    );
}

#[test]
fn test_substitution_order_and_overwrite() {
    let subs = build_substitutions(
        "deadbeef",
        &[
            ("%FOO%".to_string(), "first".to_string()),
            ("%BAR%".to_string(), "bar_val".to_string()),
            ("%FOO%".to_string(), "second".to_string()),
        ],
    );
    assert_eq!(
        subs,
        vec![
            ("%APP_HASH%".to_string(), "deadbeef".to_string()),
            ("%FOO%".to_string(), "second".to_string()),
            ("%BAR%".to_string(), "bar_val".to_string()),
        ]
    );

    let tmp = TempDir::new("subst_order");
    let tpl = tmp.path().join("tpl.txt");
    let out = tmp.path().join("out.txt");
    fs::write(&tpl, "%APP_HASH% %FOO% %BAR%").unwrap();
    expand_template(&tpl, &out, &subs).unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "deadbeef second bar_val");
}

#[cfg(unix)]
#[test]
fn test_is_symlink_zero_vs_one_on_symlink() {
    let tmp = TempDir::new("symlink_zero_vs_one");
    let target_file = tmp.path().join("target.txt");
    fs::write(&target_file, b"target_file_payload").unwrap();

    let sym = tmp.path().join("sym.txt");
    create_symlink(&target_file, &sym);

    // When is_symlink == "0", it opens `sym` (following the symlink) and hashes the target file's bytes.
    let manifest_0 = tmp.path().join("manifest_0.txt");
    let line_0 = format!("rf-file|0|sym.txt|{}", sym.display());
    fs::write(&manifest_0, format!("{}\n", line_0)).unwrap();

    let hash_0 = compute_inputs_hash(&manifest_0).unwrap();
    let mut h0 = Sha256::new();
    h0.update(line_0.split_once('|').unwrap().1.as_bytes());
    h0.update(b"target_file_payload");
    assert_eq!(hash_0, hex_encode(&h0.finalize()));

    // When is_symlink == "1", it reads the symlink target path via readlink and hashes the path bytes.
    let manifest_1 = tmp.path().join("manifest_1.txt");
    let line_1 = format!("rf-symlink|1|sym.txt|{}", sym.display());
    fs::write(&manifest_1, format!("{}\n", line_1)).unwrap();

    let hash_1 = compute_inputs_hash(&manifest_1).unwrap();
    let mut h1 = Sha256::new();
    h1.update(line_1.split_once('|').unwrap().1.as_bytes());
    h1.update(target_file.to_string_lossy().as_bytes());
    assert_eq!(hash_1, hex_encode(&h1.finalize()));
    assert_ne!(hash_0, hash_1);
}

#[test]
fn test_large_file_streaming_and_crlf_manifest() {
    let tmp = TempDir::new("large_file_crlf");
    let large_file = tmp.path().join("large.bin");
    // 600 KiB > BLOCK_SIZE (256 KiB) to exercise multi-chunk reading
    let payload: Vec<u8> = (0..(600 * 1024)).map(|i| (i % 251) as u8).collect();
    fs::write(&large_file, &payload).unwrap();

    let line1 = format!("rf-file|0|large.bin|{}", large_file.display());
    let line2 = "rf-empty|empty.py".to_string();

    let manifest_crlf = tmp.path().join("manifest_crlf.txt");
    fs::write(&manifest_crlf, format!("{}\r\n{}\r\n", line1, line2)).unwrap();

    let manifest_lf = tmp.path().join("manifest_lf.txt");
    fs::write(&manifest_lf, format!("{}\n{}\n", line1, line2)).unwrap();

    let hash_crlf = compute_inputs_hash(&manifest_crlf).unwrap();
    let hash_lf = compute_inputs_hash(&manifest_lf).unwrap();
    assert_eq!(hash_crlf, hash_lf);

    let mut expected = Sha256::new();
    // "rf-empty|empty.py" sorts before "rf-file|0|large.bin|..."
    expected.update(b"empty.py");
    expected.update(line1.split_once('|').unwrap().1.as_bytes());
    expected.update(&payload);
    assert_eq!(hash_lf, hex_encode(&expected.finalize()));
}

#[test]
fn test_parse_args_errors() {
    // Missing required flags
    assert!(parse_args::<_, &str>([]).is_err());
    assert!(parse_args(["--template=a", "--output=b"]).is_err());

    // Invalid substitution without '='
    assert!(
        parse_args([
            "--template=a",
            "--output=b",
            "--hash_files_manifest=c",
            "--substitution=NO_EQUALS",
        ])
        .is_err()
    );

    // Unknown flag
    assert!(
        parse_args([
            "--template=a",
            "--output=b",
            "--hash_files_manifest=c",
            "--unknown=1",
        ])
        .is_err()
    );

    // Unexpected positional arg
    assert!(
        parse_args([
            "--template=a",
            "--output=b",
            "--hash_files_manifest=c",
            "extra"
        ])
        .is_err()
    );

    assert_eq!(run_cli::<_, &str>([]), ExitCode::FAILURE);
}
