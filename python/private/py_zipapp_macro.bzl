# Copyright 2024 The Bazel Authors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""`py_zipapp_binary` macro."""

load("//python:py_binary.bzl", "py_binary")
load(":py_zipapp_rule.bzl", "py_zipapp_binary_rule")

def py_zipapp_binary(name, **kwargs):
    """Builds a self-contained, executable Python zip archive.

    This rule is a wrapper around `py_binary` that produces a zipapp file.
    The zipapp format is a self-contained executable zip archive that contains
    the Python code and its dependencies.

    Args:
        name: The name of the target.
        **kwargs: Arguments passed to the underlying `py_binary` rule.
    """
    py_binary_name = name + "_py_binary"
    py_binary(
        name = py_binary_name,
        **kwargs
    )

    py_zipapp_binary_rule(
        name = name,
        py_binary = py_binary_name,
        srcs = kwargs.get("srcs"),
        deps = kwargs.get("deps"),
        visibility = kwargs.get("visibility"),
        tags = kwargs.get("tags"),
        testonly = kwargs.get("testonly"),
    )
