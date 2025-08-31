# Copyright 2025 The Bazel Authors. All rights reserved.
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

"Python toolchain module extension for internal rule use"

load(":internal_config_repo.bzl", "internal_config_repo")
load("//python/private/pypi:deps.bzl", "pypi_deps")

def _internal_deps_impl(mctx):
    extra_labels = [
        str(tag.setting)
        for mod in mctx.modules
        for tag in mod.tags.add_extra_transition_setting
    ]

    internal_config_repo(
        name = "rules_python_internal",
        extra_transition_labels = extra_labels,
    )
    pypi_deps()

add_extra_transition_setting = tag_class(
    attrs = {"setting": attr.label()},
)

internal_deps = module_extension(
    implementation = _internal_deps_impl,
    tag_classes = {"add_extra_transition_setting": add_extra_transition_setting},
    doc = "This extension registers internal rules_python dependencies.",
)
