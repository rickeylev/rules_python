"""Unit tests for the WORKSPACE uv_lock macro."""

load("@rules_testing//lib:test_suite.bzl", "test_suite")
load("//python:pip.bzl", "uv_lock")
load("//python/private/pypi:hash.bzl", "hash")  # buildifier: disable=bzl-visibility
load("//python/private/pypi:pip_repository.bzl", "select_workspace_packages")  # buildifier: disable=bzl-visibility
load("//tests/support/mocks:mocks.bzl", "mocks")

_tests = []

def _test_single_version(env):
    mock_repo = mocks.repo()
    mock_hub = mocks.repo()

    uv_lock(
        name = "pypi",
        python_interpreter_target = "@python3_14_host//:python",
        uv_lock = "//:uv.lock",
        extra_pip_args = ["--retries=5"],
        _uv_lock_repo = mock_repo.rule,
        _multi_pip_parse_hub_repo = mock_hub.rule,
    )

    env.expect.that_collection(mock_hub.calls).contains_exactly([])
    env.expect.that_collection(mock_repo.calls).contains_exactly([
        {
            "extra_pip_args": ["--retries=5"],
            "name": "pypi",
            "python_interpreter_target": "@python3_14_host//:python",
            "uv_lock": "//:uv.lock",
        },
    ])

_tests.append(_test_single_version)

def _test_multi_version(env):
    mock_repo = mocks.repo()
    mock_hub = mocks.repo()

    uv_lock(
        name = "pypi",
        default_version = "3.10",
        minor_mapping = {
            "3.10": "3.10.20",
            "3.11": "3.11.15",
        },
        python_interpreter_target = {
            "3.10": "@python_3_10_host//:python",
            "3.11": "@python_3_11_host//:python",
        },
        uv_lock = "//requirements:uv.lock",
        _uv_lock_repo = mock_repo.rule,
        _multi_pip_parse_hub_repo = mock_hub.rule,
    )

    env.expect.that_collection(mock_repo.calls).contains_exactly([
        {
            "name": "pypi_3_10",
            "python_interpreter_target": "@python_3_10_host//:python",
            "uv_lock": "//requirements:uv.lock",
        },
        {
            "name": "pypi_3_11",
            "python_interpreter_target": "@python_3_11_host//:python",
            "uv_lock": "//requirements:uv.lock",
        },
    ])
    env.expect.that_collection(mock_hub.calls).contains_exactly([
        {
            "default_version": "3.10",
            "minor_mapping": {
                "3.10": "3.10.20",
                "3.11": "3.11.15",
            },
            "name": "pypi",
            "pip_parses": {
                "3.10": "pypi_3_10",
                "3.11": "pypi_3_11",
            },
        },
    ])

_tests.append(_test_multi_version)

def _test_select_workspace_packages_bazel_downloader(env):
    """Verify `.whl` entries use Bazel downloader kwargs while sdist/git fall back to pip."""
    digest_hex = "035c1e85572b141bf1c3a3eff0e33a8eb74917522f2559075ff8d2fea780a1ae"
    sdist_hex = "feedcafe0123456789abcdef0123456789abcdef0123456789abcdef01234567"
    requirements_by_platform = [
        struct(
            name = "certifi",
            is_exposed = True,
            srcs = [
                struct(
                    distribution = "certifi",
                    extra_pip_args = [],
                    requirement_line = "certifi==2026.10.19",
                    target_platforms = ["linux_x86_64"],
                    filename = "certifi-2026.10.19-py3-none-any.whl",
                    digest = "sha256:{}".format(digest_hex),
                    url = "https://files.pythonhosted.org/packages/certifi-2026.10.19-py3-none-any.whl",
                    yanked = None,
                ),
            ],
        ),
        struct(
            name = "sdist_only",
            is_exposed = True,
            srcs = [
                struct(
                    distribution = "sdist-only",
                    extra_pip_args = ["--retries=3"],
                    requirement_line = "sdist-only==1.0.0",
                    target_platforms = ["linux_x86_64"],
                    filename = "sdist_only-1.0.0.tar.gz",
                    digest = "sha256:{}".format(sdist_hex),
                    url = "https://files.pythonhosted.org/packages/sdist_only-1.0.0.tar.gz",
                    yanked = None,
                ),
            ],
        ),
        struct(
            name = "git_pkg",
            is_exposed = True,
            srcs = [
                struct(
                    distribution = "git-pkg",
                    extra_pip_args = [],
                    requirement_line = "git-pkg @ git+https://github.com/org/repo.git@deadbeef",
                    target_platforms = ["linux_x86_64"],
                    filename = "",
                    digest = "",
                    url = "",
                    yanked = None,
                ),
            ],
        ),
        struct(
            name = "virtual_root",
            is_exposed = False,
            srcs = [],
        ),
    ]

    bzl_packages, packages, options = select_workspace_packages(
        requirements_by_platform,
        repo_name = "pypi",
        download_only = False,
        repository_platform = "linux_x86_64",
    )

    env.expect.that_collection(bzl_packages).contains_exactly([
        "certifi",
        "git_pkg",
        "sdist_only",
    ])
    env.expect.that_collection(options).contains_exactly(["--retries=3"])
    env.expect.that_collection(packages).contains_exactly([
        (
            "pypi_certifi",
            "certifi==2026.10.19",
            {
                "filename": "certifi-2026.10.19-py3-none-any.whl",
                "integrity": hash.integrity("sha256:{}".format(digest_hex)),
                "urls": ["https://files.pythonhosted.org/packages/certifi-2026.10.19-py3-none-any.whl"],
            },
        ),
        (
            "pypi_git_pkg",
            "git-pkg @ git+https://github.com/org/repo.git@deadbeef",
        ),
        (
            "pypi_sdist_only",
            "sdist-only==1.0.0 --hash=sha256:{}".format(sdist_hex),
        ),
    ])

_tests.append(_test_select_workspace_packages_bazel_downloader)

def _test_select_workspace_packages_cross_platform(env):
    """Verify multi-platform wheels emit host-independent `platform_whls`."""
    linux_hex = "035c1e85572b141bf1c3a3eff0e33a8eb74917522f2559075ff8d2fea780a1ae"
    osx_hex = "feedcafe0123456789abcdef0123456789abcdef0123456789abcdef01234567"
    requirements_by_platform = [
        struct(
            name = "charset_normalizer",
            is_exposed = True,
            srcs = [
                struct(
                    distribution = "charset-normalizer",
                    extra_pip_args = [],
                    requirement_line = "charset-normalizer==3.5.2",
                    target_platforms = ["linux_x86_64"],
                    filename = "charset_normalizer-3.5.2-cp37-abi3-manylinux1_x86_64.whl",
                    digest = "sha256:{}".format(linux_hex),
                    url = "https://files.pythonhosted.org/packages/charset_normalizer-3.5.2-cp37-abi3-manylinux1_x86_64.whl",
                    yanked = None,
                ),
                struct(
                    distribution = "charset-normalizer",
                    extra_pip_args = [],
                    requirement_line = "charset-normalizer==3.5.2",
                    target_platforms = ["osx_aarch64", "osx_x86_64"],
                    filename = "charset_normalizer-3.5.2-cp37-abi3-macosx_10_9_universal2.whl",
                    digest = "sha256:{}".format(osx_hex),
                    url = "https://files.pythonhosted.org/packages/charset_normalizer-3.5.2-cp37-abi3-macosx_10_9_universal2.whl",
                    yanked = None,
                ),
            ],
        ),
    ]

    bzl_packages, packages, _ = select_workspace_packages(
        requirements_by_platform,
        repo_name = "pypi",
        download_only = False,
        repository_platform = "linux_x86_64",
        cross_platform = True,
    )

    env.expect.that_collection(bzl_packages).contains_exactly(["charset_normalizer"])
    env.expect.that_collection(packages).contains_exactly([
        (
            "pypi_charset_normalizer",
            "charset-normalizer==3.5.2",
            {
                "platform_whls": {
                    "linux_x86_64": json.encode({
                        "filename": "charset_normalizer-3.5.2-cp37-abi3-manylinux1_x86_64.whl",
                        "integrity": hash.integrity("sha256:{}".format(linux_hex)),
                        "urls": ["https://files.pythonhosted.org/packages/charset_normalizer-3.5.2-cp37-abi3-manylinux1_x86_64.whl"],
                    }),
                    "osx_aarch64": json.encode({
                        "filename": "charset_normalizer-3.5.2-cp37-abi3-macosx_10_9_universal2.whl",
                        "integrity": hash.integrity("sha256:{}".format(osx_hex)),
                        "urls": ["https://files.pythonhosted.org/packages/charset_normalizer-3.5.2-cp37-abi3-macosx_10_9_universal2.whl"],
                    }),
                    "osx_x86_64": json.encode({
                        "filename": "charset_normalizer-3.5.2-cp37-abi3-macosx_10_9_universal2.whl",
                        "integrity": hash.integrity("sha256:{}".format(osx_hex)),
                        "urls": ["https://files.pythonhosted.org/packages/charset_normalizer-3.5.2-cp37-abi3-macosx_10_9_universal2.whl"],
                    }),
                },
            },
        ),
    ])

_tests.append(_test_select_workspace_packages_cross_platform)

def uv_lock_test_suite(name):
    """Create the test suite.

    Args:
        name: the name of the test suite
    """
    test_suite(name = name, basic_tests = _tests)
