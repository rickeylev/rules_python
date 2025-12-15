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

import os
import shutil
import tempfile
import unittest
import zipfile

from tools.zipper import zipper


class ZipperTest(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.manifest_path = os.path.join(self.test_dir, "manifest.txt")
        self.output_zip = os.path.join(self.test_dir, "output.zip")

    def tearDown(self):
        shutil.rmtree(self.test_dir)

    def test_create_zip_with_files_and_symlinks(self):
        # Create some content files
        file1_path = os.path.join(self.test_dir, "file1.txt")
        with open(file1_path, "w") as f:
            f.write("content1")

        # Create a symlink
        # Note: os.symlink might fail on Windows without privileges, but we are on Linux.
        link_target_path = "target.txt"  # Relative target
        symlink_path = os.path.join(self.test_dir, "symlink_source")
        os.symlink(link_target_path, symlink_path)

        # Create a file that acts as a symlink definition (text file)
        fake_symlink_path = os.path.join(self.test_dir, "fake_symlink.txt")
        with open(fake_symlink_path, "w") as f:
            f.write("target/in/zip")

        # Prepare manifest
        # 1. Regular file
        # 2. Symlink (pointing to file on disk - reads target)
        # 3. Fake symlink (text file content - reads content)

        manifest_content = [
            f"0|file1.txt|{file1_path}",
            f"1|link1|{symlink_path}",  # Should read target 'target.txt'
        ]

        with open(self.manifest_path, "w") as f:
            f.write("\n".join(manifest_content))

        # Run zipper
        zipper.create_zip(self.manifest_path, self.output_zip)

        # Verify
        self.assertTrue(os.path.exists(self.output_zip))

        with zipfile.ZipFile(self.output_zip, "r") as zf:
            # Check file1
            info1 = zf.getinfo("file1.txt")
            self.assertFalse(self.is_symlink(info1))
            self.assertEqual(zf.read("file1.txt").decode(), "content1")

            # Check link1
            info2 = zf.getinfo("link1")
            self.assertTrue(
                self.is_symlink(info2), f"link1 attr: {hex(info2.external_attr)}"
            )
            # The content should be the target path
            self.assertEqual(zf.read("link1").decode(), "target.txt")

    def is_symlink(self, zip_info):
        # Check upper 4 bits of external_attr for S_IFLNK
        # S_IFLNK is 0o120000 = 0xA000
        attr = zip_info.external_attr >> 16
        return (attr & 0xF000) == 0xA000


if __name__ == "__main__":
    unittest.main()
