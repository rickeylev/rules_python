import unittest
import os
from python.runfiles import runfiles

class ZippedStdlibTest(unittest.TestCase):
    def test_zip_file_exists(self):
        r = runfiles.Create()
        repo = r.CurrentRepository() or "_main"
        
        # Determine the full correct runfiles structure.
        # Runfiles api returns absolute paths.
        zip_path = r.Rlocation(f"{repo}/tests/py_runtime/zipped_runtime/lib/python3.9/python3.9.zip")
        self.assertIsNotNone(zip_path)
        
        # Verify it actually exists
        self.assertTrue(os.path.exists(zip_path))
        with open(zip_path, "rb") as f:
            self.assertTrue(f.read(4).startswith(b"PK\x03\x04")) # ZIP magic bytes

    def test_original_files_omitted(self):
        r = runfiles.Create()
        repo = r.CurrentRepository() or "_main"
        
        os_py_path = r.Rlocation(f"{repo}/tests/py_runtime/zipped_runtime/lib/python3.9/os.py")
        
        # Wait, if `declare_file` is used, the generated file sits in bazel-out,
        # but in `runfiles` it might just not exist.
        if os_py_path is not None:
            self.assertFalse(os.path.exists(os_py_path))

if __name__ == "__main__":
    unittest.main()
