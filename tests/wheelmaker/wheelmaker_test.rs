use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use wheelmaker_lib::{
    arcname_from, build_wheel, commonpath, escape_filename_distribution_name,
    escape_filename_segment, get_new_requirement_line, normalize_package_name, normalize_pep440,
    parse_args, quote_filename, resolve_argument_stamp, run, serialize_digest, WhlFile,
    REGULAR_FILE_UNIX_MODE,
};
use zip::{CompressionMethod, DateTime, ZipArchive};

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = env::temp_dir().join(format!(
            "wheelmaker_test_{}_{}_{}",
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

#[derive(Debug, Clone)]
struct ParsedZipEntry {
    name: String,
    compression: CompressionMethod,
    last_modified: Option<DateTime>,
    unix_mode: Option<u32>,
    file_size: u64,
    data: Vec<u8>,
}

impl ParsedZipEntry {
    fn text(&self) -> String {
        String::from_utf8(self.data.clone()).unwrap()
    }
}

fn read_zip_entries(zip_path: &Path) -> Vec<ParsedZipEntry> {
    let file = fs::File::open(zip_path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut entries = Vec::with_capacity(archive.len());

    for idx in 0..archive.len() {
        let mut entry = archive.by_index(idx).unwrap();
        let name = entry.name().to_string();
        let compression = entry.compression();
        let last_modified = entry.last_modified();
        let unix_mode = entry.unix_mode();
        let file_size = entry.size();
        let mut data = Vec::with_capacity(file_size as usize);
        entry.read_to_end(&mut data).unwrap();
        assert_eq!(
            data.len() as u64,
            file_size,
            "uncompressed size mismatch for {}",
            name
        );

        entries.push(ParsedZipEntry {
            name,
            compression,
            last_modified,
            unix_mode,
            file_size,
            data,
        });
    }

    entries
}

fn assert_reproducible_metadata(entries: &[ParsedZipEntry], expected_method: CompressionMethod) {
    for entry in entries {
        assert_eq!(
            entry.last_modified,
            Some(DateTime::default()),
            "{}",
            entry.name
        );
        assert_eq!(
            entry.unix_mode,
            Some(REGULAR_FILE_UNIX_MODE),
            "{}",
            entry.name
        );
        assert_eq!(entry.compression, expected_method, "{}", entry.name);
        assert_eq!(entry.file_size, entry.data.len() as u64, "{}", entry.name);
    }
}

#[test]
fn test_quote_all_filenames() {
    let whl_true = WhlFile::new(
        Cursor::new(Vec::new()),
        "test-1.0.0",
        Vec::new(),
        "",
        true,
        true,
    );
    assert_eq!(whl_true.quote_filename("foo/bar.py"), "\"foo/bar.py\"");
    assert_eq!(
        whl_true.quote_filename("foo,bar/baz.py"),
        "\"foo,bar/baz.py\""
    );

    let whl_false = WhlFile::new(
        Cursor::new(Vec::new()),
        "test-1.0.0",
        Vec::new(),
        "",
        true,
        false,
    );
    assert_eq!(whl_false.quote_filename("foo/bar.py"), "foo/bar.py");
    assert_eq!(
        whl_false.quote_filename("foo,bar/baz.py"),
        "\"foo,bar/baz.py\""
    );
    assert_eq!(
        quote_filename("/leading/slash.py", false),
        "leading/slash.py"
    );
    assert_eq!(
        quote_filename("has\"quote.py", false),
        "\"has\"\"quote.py\""
    );
}

#[test]
fn test_arcname_from() {
    struct Case<'a> {
        name: &'a str,
        distribution_prefix: &'a str,
        strip_path_prefixes: &'a [&'a str],
        add_path_prefix: &'a str,
        expected: &'a str,
    }

    let cases = [
        Case {
            name: "a/b/c/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &[],
            add_path_prefix: "",
            expected: "a/b/c/file.py",
        },
        Case {
            name: "a/b/c/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &["a"],
            add_path_prefix: "",
            expected: "/b/c/file.py",
        },
        Case {
            name: "a/b/c/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &["a/b/"],
            add_path_prefix: "",
            expected: "c/file.py",
        },
        Case {
            name: "a/b/c/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &["a/", "b/"],
            add_path_prefix: "",
            expected: "b/c/file.py",
        },
        Case {
            name: "foo/bar/baz/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &["foo", "foo/bar/baz"],
            add_path_prefix: "",
            expected: "/bar/baz/file.py",
        },
        Case {
            name: "foo/bar/baz/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &["foo/bar/baz", "foo"],
            add_path_prefix: "",
            expected: "/file.py",
        },
        Case {
            name: "foo/file2.py",
            distribution_prefix: "",
            strip_path_prefixes: &["foo/bar/baz", "foo"],
            add_path_prefix: "",
            expected: "/file2.py",
        },
        Case {
            name: "mylib-0.0.1-dist-info/WHEEL",
            distribution_prefix: "mylib",
            strip_path_prefixes: &[],
            add_path_prefix: "",
            expected: "mylib-0.0.1-dist-info/WHEEL",
        },
        Case {
            name: "mylib/a/b/c/WHEEL",
            distribution_prefix: "mylib",
            strip_path_prefixes: &["mylib"],
            add_path_prefix: "",
            expected: "mylib/a/b/c/WHEEL",
        },
        Case {
            name: "a/b/c/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &[],
            add_path_prefix: "namespace/",
            expected: "namespace/a/b/c/file.py",
        },
        Case {
            name: "a/b/c/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &["a"],
            add_path_prefix: "namespace",
            expected: "namespace/b/c/file.py",
        },
        Case {
            name: "a/b/c/file.py",
            distribution_prefix: "",
            strip_path_prefixes: &["a/b/"],
            add_path_prefix: "namespace_",
            expected: "namespace_c/file.py",
        },
    ];

    for c in cases {
        let prefixes: Vec<String> = c
            .strip_path_prefixes
            .iter()
            .map(|s| s.to_string())
            .collect();
        let got = arcname_from(c.name, c.distribution_prefix, &prefixes, c.add_path_prefix);
        assert_eq!(got, c.expected, "case: {}", c.name);
    }
}

#[test]
fn test_get_new_requirement_line() {
    assert_eq!(
        get_new_requirement_line("requests>=2.0", "").unwrap(),
        "Requires-Dist: requests>=2.0"
    );
    assert_eq!(
        get_new_requirement_line("requests>=2.0", "extra=='dev'").unwrap(),
        "Requires-Dist: requests>=2.0; extra=='dev'"
    );
    assert_eq!(
        get_new_requirement_line(
            "requests @ git+https://github.com/psf/requests.git@3aa6386c3",
            ""
        )
        .unwrap(),
        "Requires-Dist: requests @ git+https://github.com/psf/requests.git@3aa6386c3"
    );
    assert_eq!(
        get_new_requirement_line("requests>=2.0; python_version>='3.6'", "").unwrap(),
        "Requires-Dist: requests>=2.0; python_version >= \"3.6\""
    );
    assert_eq!(
        get_new_requirement_line("requests>=2.0; python_version>='3.6'", "extra=='dev'").unwrap(),
        "Requires-Dist: requests>=2.0; (python_version >= \"3.6\") and extra=='dev'"
    );
    assert_eq!(
        get_new_requirement_line("pyyaml>=6.0.0,!=6.0.1", "extra == 'example'").unwrap(),
        "Requires-Dist: pyyaml!=6.0.1,>=6.0.0; extra == 'example'"
    );
    assert_eq!(
        get_new_requirement_line(
            "toml; (python_version == \"3.11\" or python_version == \"3.12\") and python_version != \"3.8\"",
            "extra == 'example'"
        )
        .unwrap(),
        "Requires-Dist: toml; ((python_version == \"3.11\" or python_version == \"3.12\") and python_version != \"3.8\") and extra == 'example'"
    );
    assert_eq!(
        get_new_requirement_line(
            "wheel; python_version == \"3.11\" or python_version == \"3.12\" ",
            "extra == 'example'"
        )
        .unwrap(),
        "Requires-Dist: wheel; (python_version == \"3.11\" or python_version == \"3.12\") and extra == 'example'"
    );
    assert_eq!(
        get_new_requirement_line("extra_requires[example]==0.0.1", "").unwrap(),
        "Requires-Dist: extra_requires[example]==0.0.1"
    );
    assert_eq!(
        get_new_requirement_line(
            "baz~=1.2.3; extra == 'test_extra' or 'dev.extra' == extra",
            ""
        )
        .unwrap(),
        "Requires-Dist: baz~=1.2.3; extra == \"test-extra\" or \"dev-extra\" == extra"
    );
    assert_eq!(
        get_new_requirement_line(
            "baz~=1.2.3; extra == 'test_extra' or 'dev.extra' == extra",
            "extra == 'dev.extra'"
        )
        .unwrap(),
        "Requires-Dist: baz~=1.2.3; (extra == \"test-extra\" or \"dev-extra\" == extra) and extra == 'dev.extra'"
    );
}

#[test]
fn test_escaping_and_pep440_normalization() {
    assert_eq!(
        normalize_package_name("File--Name_Escaping.Pkg"),
        "file-name-escaping-pkg"
    );
    assert_eq!(
        escape_filename_distribution_name("File--Name-Escaping"),
        "file_name_escaping"
    );
    assert_eq!(escape_filename_segment("py2.py3-none"), "py2.py3_none");

    assert_eq!(
        normalize_pep440("v0.0.1.RC1+ubuntu-r7"),
        "0.0.1rc1+ubuntu.r7"
    );
    assert_eq!(normalize_pep440("01.02.003"), "1.2.3");
    assert_eq!(normalize_pep440("1.2.3.dev0"), "1.2.3.dev0");
    assert_eq!(normalize_pep440("1.0-2"), "1.0.post2");
    assert_eq!(normalize_pep440("1!2.0.rev3"), "1!2.0.post3");
    assert_eq!(
        normalize_pep440("0.1.{BUILD_TIMESTAMP}"),
        "0.1.0+0.1.build.timestamp"
    );
    assert_eq!(normalize_pep440("invalid_ver"), "0+invalid.ver");
}

#[test]
fn test_end_to_end_customized_wheel_and_record() {
    let tmp = TempDir::new("customized");
    let data_commas = tmp.path().join("data,with,commas.txt");
    fs::write(&data_commas, b"hello world\n").unwrap();

    let empty_pyi = tmp.path().join("mod.pyi");
    fs::write(&empty_pyi, b"").unwrap();

    let sub_dir = tmp.path().join("someDir");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(sub_dir.join("foo.py"), b"x = 1\n").unwrap();

    let input_list_path = tmp.path().join("inputs.txt");
    fs::write(
        &input_list_path,
        format!(
            "examples/wheel/lib/data,with,commas.txt;{}\nexamples/wheel/lib/mod.pyi;{}\nexamples/wheel/someDir;{}\n",
            data_commas.display(),
            empty_pyi.display(),
            sub_dir.display(),
        ),
    )
    .unwrap();

    let reqs_file = tmp.path().join("requires.txt");
    fs::write(
        &reqs_file,
        "# comment\n--index-url https://pypi.org\n\ntomli>=2.0.0\nstarlark  # inline comment\n",
    )
    .unwrap();

    let extra_reqs_file = tmp.path().join("extra_requires.txt");
    fs::write(
        &extra_reqs_file,
        "pyyaml>=6.0.0,!=6.0.1\ntoml; (python_version == \"3.11\" or python_version == \"3.12\") and python_version != \"3.8\"\n",
    )
    .unwrap();

    let metadata_file = tmp.path().join("metadata.txt");
    fs::write(
        &metadata_file,
        format!(
            "Metadata-Version: 2.1\nName: File--Name-Escaping\nAuthor: Example Author: żółw\nRequires-Dist: @{}\nProvides-Extra: example\nRequires-Dist: @{}; extra == 'example'\n",
            reqs_file.display(),
            extra_reqs_file.display(),
        ),
    )
    .unwrap();

    let desc_file = tmp.path().join("README.md");
    fs::write(&desc_file, b"This is a sample description of a wheel.").unwrap();

    let ep_file = tmp.path().join("entry_points.txt");
    fs::write(&ep_file, b"[console_scripts]\nmain = foo.bar:baz").unwrap();

    let notice_file = tmp.path().join("NOTICE");
    fs::write(&notice_file, b"Notice text\n").unwrap();

    let out_whl = tmp.path().join("out.whl");
    let name_file = tmp.path().join("out.name");

    let opts = parse_args([
        "--name",
        "File--Name-Escaping",
        "--version",
        "v0.0.1.RC1+ubuntu-r7",
        "--python_tag",
        "py3",
        "--abi",
        "none",
        "--platform",
        "any",
        "--out",
        out_whl.to_str().unwrap(),
        "--name_file",
        name_file.to_str().unwrap(),
        "--strip_path_prefix=examples/wheel/",
        "--path_prefix=pkg/",
        "--input_file_list",
        input_list_path.to_str().unwrap(),
        "--metadata_file",
        metadata_file.to_str().unwrap(),
        "--description_file",
        desc_file.to_str().unwrap(),
        "--entry_points_file",
        ep_file.to_str().unwrap(),
        "--extra_distinfo_file",
        &format!("NOTICE;{}", notice_file.display()),
        "--data_files",
        &format!("scripts/NOTICE;{}", notice_file.display()),
    ])
    .unwrap();

    build_wheel(&opts).unwrap();

    assert_eq!(
        fs::read_to_string(&name_file).unwrap(),
        "file_name_escaping-0.0.1rc1+ubuntu.r7-py3-none-any.whl"
    );

    let entries = read_zip_entries(&out_whl);
    assert_reproducible_metadata(&entries, CompressionMethod::Deflated);

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "pkg/lib/data,with,commas.txt",
            "pkg/lib/mod.pyi",
            "pkg/someDir/foo.py",
            "file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/WHEEL",
            "file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/METADATA",
            "file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/entry_points.txt",
            "file_name_escaping-0.0.1rc1+ubuntu.r7.data/scripts/NOTICE",
            "file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/NOTICE",
            "file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/RECORD",
        ]
    );

    let by_name: HashMap<String, ParsedZipEntry> =
        entries.into_iter().map(|e| (e.name.clone(), e)).collect();

    assert_eq!(
        by_name["file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/WHEEL"].text(),
        "Wheel-Version: 1.0\nGenerator: bazel-wheelmaker 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
    );

    assert_eq!(
        by_name["file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/METADATA"].text(),
        concat!(
            "Metadata-Version: 2.1\n",
            "Name: File--Name-Escaping\n",
            "Author: Example Author: żółw\n",
            "Requires-Dist: tomli>=2.0.0\n",
            "Requires-Dist: starlark\n",
            "Provides-Extra: example\n",
            "Requires-Dist: pyyaml!=6.0.1,>=6.0.0; extra == 'example'\n",
            "Requires-Dist: toml; ((python_version == \"3.11\" or python_version == \"3.12\") and python_version != \"3.8\") and extra == 'example'\n",
            "Version: 0.0.1rc1+ubuntu.r7\n\n",
            "This is a sample description of a wheel.\n",
        )
    );

    let record_text = by_name["file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/RECORD"].text();
    let record_lines: Vec<&str> = record_text.lines().collect();
    assert_eq!(record_lines.len(), 9);
    let mut sha = Sha256::new();
    sha.update(b"hello world\n");
    let expected_first = format!(
        "\"pkg/lib/data,with,commas.txt\",{},12",
        serialize_digest(&sha.finalize())
    );
    assert_eq!(record_lines[0], expected_first);
    assert_eq!(
        record_lines[8],
        "file_name_escaping-0.0.1rc1+ubuntu.r7.dist-info/RECORD,,"
    );
}

#[test]
fn test_stamping_and_empty_requires_and_no_compress() {
    let tmp = TempDir::new("stamping");
    let volatile_file = tmp.path().join("volatile-status.txt");
    fs::write(&volatile_file, "BUILD_TIMESTAMP 123456\n").unwrap();

    let stable_file = tmp.path().join("stable-status.txt");
    fs::write(&stable_file, "BUILD_USER alice\n").unwrap();

    assert_eq!(
        resolve_argument_stamp(
            "pkg_{BUILD_USER}_{BUILD_TIMESTAMP}",
            &volatile_file,
            &stable_file
        )
        .unwrap(),
        "pkg_alice_123456"
    );

    let empty_reqs = tmp.path().join("empty_requires.txt");
    fs::write(&empty_reqs, "\n# only comments\n").unwrap();

    let metadata_file = tmp.path().join("metadata.txt");
    fs::write(
        &metadata_file,
        format!(
            "Metadata-Version: 2.1\nName: my_pkg{{BUILD_USER}}\nRequires-Dist: @{}\n",
            empty_reqs.display()
        ),
    )
    .unwrap();

    let out_whl = tmp.path().join("stamped.whl");
    let name_file = tmp.path().join("stamped.name");
    let flagfile = tmp.path().join("flags.params");
    fs::write(
        &flagfile,
        format!(
            "--name\nmy_pkg{{BUILD_USER}}\n--version\n0.1.{{BUILD_TIMESTAMP}}\n--platform\nmanylinux2014_x86_64\n--abi\nabi3\n--python_tag\ncp38\n--no_compress\n--out\n{}\n--name_file\n{}\n--metadata_file\n{}\n--volatile_status_file\n{}\n--stable_status_file\n{}\n",
            out_whl.display(),
            name_file.display(),
            metadata_file.display(),
            volatile_file.display(),
            stable_file.display(),
        ),
    )
    .unwrap();

    run([format!("@{}", flagfile.display())]).unwrap();

    assert_eq!(
        fs::read_to_string(&name_file).unwrap(),
        "my_pkgalice-0.1.123456-cp38-abi3-manylinux2014_x86_64.whl"
    );

    let entries = read_zip_entries(&out_whl);
    assert_reproducible_metadata(&entries, CompressionMethod::Stored);

    let by_name: HashMap<String, ParsedZipEntry> =
        entries.into_iter().map(|e| (e.name.clone(), e)).collect();

    assert_eq!(
        by_name["my_pkgalice-0.1.123456.dist-info/WHEEL"].text(),
        "Wheel-Version: 1.0\nGenerator: bazel-wheelmaker 1.0\nRoot-Is-Purelib: false\nTag: cp38-abi3-manylinux2014_x86_64\n"
    );

    assert_eq!(
        by_name["my_pkgalice-0.1.123456.dist-info/METADATA"].text(),
        "Metadata-Version: 2.1\nName: my_pkgalice\nVersion: 0.1.123456\n\nUNKNOWN\n"
    );
}

#[test]
fn test_build_tag_and_inline_requires_and_commonpath() {
    assert_eq!(commonpath("a/b/c", "a/b/d"), "a/b");
    assert_eq!(commonpath("a/b", "a/b/c"), "a/b");
    assert_eq!(commonpath("a/b", "x/y"), "");

    let tmp = TempDir::new("build_tag");
    let extra_reqs = tmp.path().join("extra_reqs.txt");
    fs::write(&extra_reqs, "pytest>=7.0\n").unwrap();

    let metadata_file = tmp.path().join("metadata.txt");
    fs::write(
        &metadata_file,
        format!(
            "Metadata-Version: 2.1\nName: Demo-Pkg\nRequires-Dist: six>=1.16.0,!=1.16.1; python_version>='3.8'; extra == 'dev'\nProvides-Extra: dev\nRequires-Dist: @{}; extra == 'dev'\n",
            extra_reqs.display()
        ),
    )
    .unwrap();

    let out_whl = tmp.path().join("demo.whl");
    let name_file = tmp.path().join("demo.name");

    let opts = parse_args([
        "--name",
        "Demo-Pkg",
        "--version",
        "1.0.0",
        "--build_tag",
        "1build",
        "--python_tag",
        "py2.py3",
        "--abi",
        "none",
        "--platform",
        "any",
        "--out",
        out_whl.to_str().unwrap(),
        "--name_file",
        name_file.to_str().unwrap(),
        "--metadata_file",
        metadata_file.to_str().unwrap(),
    ])
    .unwrap();

    build_wheel(&opts).unwrap();

    assert_eq!(
        fs::read_to_string(&name_file).unwrap(),
        "demo_pkg-1.0.0-1build-py2.py3-none-any.whl"
    );

    let entries = read_zip_entries(&out_whl);
    assert_reproducible_metadata(&entries, CompressionMethod::Deflated);
    let by_name: HashMap<String, ParsedZipEntry> =
        entries.into_iter().map(|e| (e.name.clone(), e)).collect();

    assert_eq!(
        by_name["demo_pkg-1.0.0.dist-info/WHEEL"].text(),
        concat!(
            "Wheel-Version: 1.0\n",
            "Generator: bazel-wheelmaker 1.0\n",
            "Root-Is-Purelib: true\n",
            "Tag: py2.py3-none-any\n",
        )
    );

    assert_eq!(
        by_name["demo_pkg-1.0.0.dist-info/METADATA"].text(),
        concat!(
            "Metadata-Version: 2.1\n",
            "Name: Demo-Pkg\n",
            "Requires-Dist: six!=1.16.1,>=1.16.0; (python_version >= \"3.8\") and extra == 'dev'\n",
            "Provides-Extra: dev\n",
            "Requires-Dist: pytest>=7.0; extra == 'dev'\n",
            "Version: 1.0.0\n\n",
            "UNKNOWN\n",
        )
    );
}
