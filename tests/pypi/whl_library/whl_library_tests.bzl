# Copyright 2024 The Bazel Authors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#    http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"Tests for //python/private/pypi:whl_library.bzl"

load("@rules_testing//lib:test_suite.bzl", "test_suite")

# buildifier: disable=bzl-visibility
load("//python/private/pypi:whl_library.bzl", "whl_library")

_tests = []

_state = {"calls": []}

def _record(rule_name, kwargs):
    _state["calls"].append(struct(
        rule = rule_name,
        kwargs = dict(kwargs),
    ))
    return kwargs.get("name")

def _reset_calls():
    _state["calls"] = []

def _mock_rules():
    return struct(
        whl_archive = lambda **kwargs: _record("whl_archive", kwargs),
        pip_archive = lambda **kwargs: _record("pip_archive", kwargs),
        whl_deps_library = lambda **kwargs: _record("whl_deps_library", kwargs),
    )

def _whl_archive_calls():
    return [c for c in _state["calls"] if c.rule == "whl_archive"]

def _whl_deps_library_calls():
    return [c for c in _state["calls"] if c.rule == "whl_deps_library"]

def _pip_archive_calls():
    return [c for c in _state["calls"] if c.rule == "pip_archive"]

def _test_whl_file_reuse_path(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        whl_file = "@some_wheels//:pkg-1.0-py3-none-any.whl",
        requirement = "pkg==1.0",
    )
    env.expect.that_int(len(_whl_archive_calls())).equals(1)
    env.expect.that_dict(_whl_archive_calls()[0].kwargs).contains_exactly({
        "name": "w_pkg_1_0_py3_none_any",
        "requirement": "pkg==1.0",
        "whl_file": "@some_wheels//:pkg-1.0-py3-none-any.whl",
    })
    env.expect.that_int(len(_whl_deps_library_calls())).equals(1)
    env.expect.that_dict(_whl_deps_library_calls()[0].kwargs).contains_exactly({
        "metadata_file": "@w_pkg_1_0_py3_none_any//:metadata.json",
        "name": "test_repo",
    })

_tests.append(_test_whl_file_reuse_path)

def _test_urls_reuse_path(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        whl_file = None,
        urls = ["https://example.com/pkg-1.0-py3-none-any.whl"],
        filename = "pkg-1.0-py3-none-any.whl",
        requirement = "pkg==1.0",
        config_load = "@pypi//:config.bzl",
    )
    env.expect.that_int(len(_whl_archive_calls())).equals(1)
    env.expect.that_dict(_whl_archive_calls()[0].kwargs).contains_exactly({
        "filename": "pkg-1.0-py3-none-any.whl",
        "name": "w_pkg_1_0_py3_none_any",
        "requirement": "pkg==1.0",
        "urls": ["https://example.com/pkg-1.0-py3-none-any.whl"],
    })
    env.expect.that_int(len(_whl_deps_library_calls())).equals(1)
    env.expect.that_dict(_whl_deps_library_calls()[0].kwargs).contains_exactly({
        "config_load": "@pypi//:config.bzl",
        "metadata_file": "@w_pkg_1_0_py3_none_any//:metadata.json",
        "name": "test_repo",
    })

_tests.append(_test_urls_reuse_path)

def _test_annotation_no_reuse(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        whl_file = None,
        urls = ["https://example.com/pkg-1.0-py3-none-any.whl"],
        filename = "pkg-1.0-py3-none-any.whl",
        requirement = "pkg[extra]==1.0",
        annotation = "@some_repo//:pkg.annotation.json",
    )
    env.expect.that_int(len(_whl_archive_calls())).equals(1)
    env.expect.that_dict(_whl_archive_calls()[0].kwargs).contains_exactly({
        "annotation": "@some_repo//:pkg.annotation.json",
        "filename": "pkg-1.0-py3-none-any.whl",
        "name": "test_repo",
        "requirement": "pkg[extra]==1.0",
        "urls": ["https://example.com/pkg-1.0-py3-none-any.whl"],
    })
    env.expect.that_int(len(_whl_deps_library_calls())).equals(0)

_tests.append(_test_annotation_no_reuse)

def _test_whl_patches_no_reuse(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        whl_file = None,
        urls = ["https://example.com/pkg-1.0-py3-none-any.whl"],
        filename = "pkg-1.0-py3-none-any.whl",
        requirement = "pkg==1.0",
        whl_patches = {"//patches:foo.patch": "json"},
    )
    env.expect.that_int(len(_whl_archive_calls())).equals(1)
    env.expect.that_dict(_whl_archive_calls()[0].kwargs).contains_exactly({
        "filename": "pkg-1.0-py3-none-any.whl",
        "name": "test_repo",
        "requirement": "pkg==1.0",
        "urls": ["https://example.com/pkg-1.0-py3-none-any.whl"],
        "whl_patches": {"//patches:foo.patch": "json"},
    })
    env.expect.that_int(len(_whl_deps_library_calls())).equals(0)

_tests.append(_test_whl_patches_no_reuse)

def _test_pip_archive_path(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        requirement = "pkg==1.0",
        config_load = "@pypi//:config.bzl",
    )
    env.expect.that_int(len(_pip_archive_calls())).equals(1)
    env.expect.that_dict(_pip_archive_calls()[0].kwargs).contains_exactly({
        "config_load": "@pypi//:config.bzl",
        "name": "test_repo",
        "requirement": "pkg==1.0",
    })
    env.expect.that_int(len(_whl_archive_calls())).equals(0)

_tests.append(_test_pip_archive_path)

def _test_extras_not_passed_when_empty(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        whl_file = None,
        urls = ["https://example.com/pkg-1.0-py3-none-any.whl"],
        filename = "pkg-1.0-py3-none-any.whl",
        requirement = "pkg==1.0",
        config_load = "@pypi//:config.bzl",
    )
    env.expect.that_int(len(_whl_deps_library_calls())).equals(1)
    env.expect.that_dict(_whl_deps_library_calls()[0].kwargs).not_exists("extras")

_tests.append(_test_extras_not_passed_when_empty)

def _test_extras_passed_when_non_empty(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        whl_file = None,
        urls = ["https://example.com/pkg-1.0-py3-none-any.whl"],
        filename = "pkg-1.0-py3-none-any.whl",
        requirement = "pkg[foo,bar]==1.0",
        config_load = "@pypi//:config.bzl",
    )
    env.expect.that_int(len(_whl_deps_library_calls())).equals(1)
    env.expect.that_list(_whl_deps_library_calls()[0].kwargs["extras"]).contains_exactly(["foo", "bar"])

_tests.append(_test_extras_passed_when_non_empty)

def _test_python_interpreter_excluded(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        whl_file = None,
        urls = ["https://example.com/pkg-1.0-py3-none-any.whl"],
        filename = "pkg-1.0-py3-none-any.whl",
        requirement = "pkg==1.0",
        config_load = "@pypi//:config.bzl",
        python_interpreter = "python3",
        python_interpreter_target = "@python//:python",
    )
    env.expect.that_int(len(_whl_archive_calls())).equals(1)
    env.expect.that_dict(_whl_archive_calls()[0].kwargs).not_exists("python_interpreter")
    env.expect.that_dict(_whl_archive_calls()[0].kwargs).not_exists("python_interpreter_target")

_tests.append(_test_python_interpreter_excluded)

def _test_config_load_excluded_from_extract_args(env):
    _reset_calls()
    whl_library(
        name = "test_repo",
        rules = _mock_rules(),
        whl_file = None,
        urls = ["https://example.com/pkg-1.0-py3-none-any.whl"],
        filename = "pkg-1.0-py3-none-any.whl",
        requirement = "pkg==1.0",
        config_load = "@pypi//:config.bzl",
        dep_template = "@pypi%{name}//:{{target}}",
    )
    env.expect.that_int(len(_whl_archive_calls())).equals(1)
    env.expect.that_dict(_whl_archive_calls()[0].kwargs).not_exists("config_load")
    env.expect.that_dict(_whl_archive_calls()[0].kwargs).not_exists("dep_template")

_tests.append(_test_config_load_excluded_from_extract_args)

def whl_library_test_suite(name):
    test_suite(
        name = name,
        basic_tests = _tests,
    )
