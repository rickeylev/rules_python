import sys
##import json
import unittest


class ZippedStdlibTest(unittest.TestCase):
    def test_stdlib_is_zipped(self):
        for x in sys.path:
            print(x)
        print(sys.path)
        import json
        # The 'json' module is part of the standard library.
        # Its __file__ attribute points to its location.
        json_path = json.__file__

        # If the stdlib is zipped, the path will often contain a '.zip' archive.
        # For example: '/path/to/python/install/lib/python3.8.zip/json/__init__.py'
        self.assertIn(".zip", json_path)


if __name__ == "__main__":
    unittest.main()
