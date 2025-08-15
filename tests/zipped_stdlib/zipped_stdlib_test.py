import os
import unittest


class ZippedStdlibTest(unittest.TestCase):
    def test_stdlib_is_not_zipped(self):
        # The 'os' module is a core part of the standard library.
        # Its __file__ attribute points to its location.
        os_path = os.__file__

        # If the stdlib is zipped, the path will often contain a '.zip' archive.
        # For example: '/path/to/python/install/lib/python3.8.zip/os.py'
        # This assertion ensures that is not the case.
        self.assertIn(".zip", os_path)


if __name__ == "__main__":
    unittest.main()
