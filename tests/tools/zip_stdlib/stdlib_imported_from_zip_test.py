import unittest
import pathlib
import os
import sys

class StdlibImportedFromZipTest(unittest.TestCase):
    def test_imports_work(self):
        self.assertTrue(hasattr(pathlib, 'Path'))
        self.assertTrue(hasattr(os, 'path'))
        
        pathlib_file = pathlib.__file__
        
        self.assertIn(".zip", pathlib_file, f"pathlib was not loaded from a zip file: {pathlib_file}")

if __name__ == "__main__":
    unittest.main()
