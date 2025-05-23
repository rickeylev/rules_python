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

"""This module contains the definitions of all providers."""

UvToolchainInfo = provider(
    doc = "Information about how to invoke the uv executable.",
    fields = {
        "label": """
:type: Label

The uv toolchain implementation label returned by the toolchain.
""",
        "uv": """
:type: Target

The uv binary `Target`
""",
        "version": """
:type: str

The uv version
""",
    },
)
