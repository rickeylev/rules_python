import unittest
import pathlib
import sys

class BasicTest(unittest.TestCase):
    def test_pathlib_is_zipped(self):
        print("SYS PATH:", sys.path)
        pathlib_file = pathlib.__file__
        print(f"pathlib is loaded from: {pathlib_file}")
        
        self.assertIn(".zip", pathlib_file, "pathlib was not loaded from a zip file.")

if __name__ == "__main__":
    unittest.main()
