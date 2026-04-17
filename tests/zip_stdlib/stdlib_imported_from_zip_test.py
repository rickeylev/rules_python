import importlib
import unittest
import sys

class StdlibImportedFromZipTest(unittest.TestCase):
    def assert_imported_from_zip(self, module_name):
        mod = importlib.import_module(module_name)
        mod_file = mod.__file__
        self.assertIn(".zip", mod_file,
                      f"'{module_name}' was not loaded from a zip file: {mod_file}\n" +
                      f"sys.path:\n" + "\n".join(sys.path))

    def test_imports_work(self):
        self.assert_imported_from_zip("pathlib")
        self.assert_imported_from_zip("os")
        self.assert_imported_from_zip("json")
        self.assert_imported_from_zip("logging")
        self.assert_imported_from_zip("shutil")
        self.assert_imported_from_zip("tarfile")
        self.assert_imported_from_zip("urllib.request")
        self.assert_imported_from_zip("concurrent.futures")
        self.assert_imported_from_zip("sqlite3")
        self.assert_imported_from_zip("re")
        self.assert_imported_from_zip("collections")

if __name__ == "__main__":
    unittest.main()
