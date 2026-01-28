import hashlib
import os
import shutil
import stat
import tempfile
import unittest

from tools.zipapp import exe_zip_maker


class ExeZipMakerTest(unittest.TestCase):
    def setUp(self):
        self.test_dir = tempfile.mkdtemp()
        self.preamble_path = os.path.join(self.test_dir, "preamble.txt")
        self.zip_path = os.path.join(self.test_dir, "data.zip")
        self.output_path = os.path.join(self.test_dir, "output.exe")

    def tearDown(self):
        shutil.rmtree(self.test_dir)

    def assertStartsWith(self, actual, expected):
        if not actual.startswith(expected):
            self.fail(f"{actual!r} does not start with {expected!r}")

    def test_create_exe_zip(self):
        # Create dummy zip file
        zip_content = b"PK\x03\x04dummyzipcontent"
        with open(self.zip_path, "wb") as f:
            f.write(zip_content)

        # Calculate expected hash
        expected_hash = hashlib.sha256(zip_content).hexdigest()

        # Create preamble with placeholder
        preamble_text = "#!/bin/bash\nEXPECTED_HASH='%ZIP_HASH%'\n# ... logic ...\n"
        with open(self.preamble_path, "w") as f:
            f.write(preamble_text)

        # Call create_exe_zip directly
        exe_zip_maker.create_exe_zip(
            self.preamble_path, self.zip_path, self.output_path
        )

        # Verify output exists
        self.assertTrue(
            os.path.exists(self.output_path),
            msg=f"Output path '{self.output_path}' should exist",
        )

        # Verify executable bit
        st = os.stat(self.output_path)
        self.assertTrue(
            st.st_mode & stat.S_IEXEC,
            msg=f"Output path '{self.output_path}' should be executable",
        )

        # Verify content
        with open(self.output_path, "rb") as f:
            content = f.read()

        # Split content back into preamble and zip
        # We know the preamble text length after substitution.
        expected_preamble = preamble_text.replace("%ZIP_HASH%", expected_hash).encode(
            "utf-8"
        )

        self.assertStartsWith(content, expected_preamble)
        self.assertTrue(
            content.endswith(zip_content),
            msg="Output content should end with the zip content",
        )
        self.assertEqual(len(content), len(expected_preamble) + len(zip_content))


if __name__ == "__main__":
    unittest.main()
