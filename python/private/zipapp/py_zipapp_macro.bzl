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

load(":py_zipapp_rule.bzl", "py_zipapp_binary_rule")

def py_zipapp_binary(name, binary, **kwargs):
    """Builds a self-contained, executable Python zip archive from an existing py_binary target.

    Args:
        name: The name of the target.
        binary: A label pointing to a py_binary target that will be packaged into the zipapp.
        **kwargs: Common attributes like visibility, tags, testonly, etc., passed to the underlying py_zipapp_binary_rule.
    """
    py_zipapp_binary_rule(
        name = name,
        binary = binary,
        **kwargs
    )
