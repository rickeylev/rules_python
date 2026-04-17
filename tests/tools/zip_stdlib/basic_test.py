import unittest
import pathlib
import os

class BasicTest(unittest.TestCase):
    def test_imports_work(self):
        self.assertTrue(hasattr(pathlib, 'Path'))
        self.assertTrue(hasattr(os, 'path'))
        
        pathlib_file = pathlib.__file__
        print(f"pathlib is loaded from: {pathlib_file}")
        self.assertIn(".zip", pathlib_file, "pathlib was not loaded from a zip file.")

if __name__ == "__main__":
    unittest.main()
