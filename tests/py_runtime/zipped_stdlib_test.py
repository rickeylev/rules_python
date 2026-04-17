import unittest
import os
import pathlib
from python.runfiles import runfiles

class ZippedStdlibTest(unittest.TestCase):
    def test_zip_file_exists(self):
        r = runfiles.Create()
        repo = r.CurrentRepository() or "_main"
        
        raw_zip_path = r.Rlocation(f"{repo}/tests/py_runtime/zipped_runtime/lib/python39.zip")
        self.assertIsNotNone(raw_zip_path, "Could not resolve zip path via Rlocation")
        
        zip_path = pathlib.Path(raw_zip_path)
        self.assertTrue(zip_path.exists())
        with zip_path.open("rb") as f:
            self.assertTrue(f.read(4).startswith(b"PK\x03\x04")) # ZIP magic bytes

    def test_original_files_omitted(self):
        r = runfiles.Create()
        repo = r.CurrentRepository() or "_main"
        
        raw_os_py_path = r.Rlocation(f"{repo}/tests/py_runtime/zipped_runtime/lib/python3.9/os.py")
        if raw_os_py_path is not None:
             os_py_path = pathlib.Path(raw_os_py_path)
             self.assertFalse(os_py_path.exists())

if __name__ == "__main__":
    unittest.main()
