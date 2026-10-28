"""Unit tests for the WORKSPACE uv_lock macro."""

load("@rules_testing//lib:test_suite.bzl", "test_suite")
load("//python:pip.bzl", "uv_lock")
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

def uv_lock_test_suite(name):
    """Create the test suite.

    Args:
        name: the name of the test suite
    """
    test_suite(name = name, basic_tests = _tests)
