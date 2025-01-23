# Copyright 2023 The Bazel Authors. All rights reserved.
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

import os
import sys

print("==============")
print("==============")
print("==============")
print("dir:", os.environ.get("COVERAGE_DIR"))
covdir = os.environ.get("COVERAGE_DIR")
print(os.path.exists(os.path.join(covdir, "pylcov.dat")))
print(sys.modules.get("STASH_COV"))
print('covrun=', os.environ.get("COVERAGE_RUN"))
print("==============")
print("==============")
sys.exit(1)

expected = os.getenv("VERSION_CHECK")
current = f"{sys.version_info.major}.{sys.version_info.minor}"

if current != expected:
    print(f"expected version '{expected}' is different than returned '{current}'")
    sys.exit(1)
