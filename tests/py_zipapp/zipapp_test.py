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

import os
import zipfile
import unittest


class PyZipAppTest(unittest.TestCase):
    def test_zipapp_contents(self):
        zipapp_path = os.environ["TEST_ZIPAPP"]

        self.assertTrue(os.path.exists(zipapp_path))
        self.assertTrue(os.path.isfile(zipapp_path))

        # The zipapp itself is a shell script prepended to the zip file.
        # We need to find the start of the actual zip data.
        with open(zipapp_path, "rb") as f:
            content = f.read()

        output = subprocess.check_output([zipapp_path])

if __name__ == "__main__":
    unittest.main()
