import os
import unittest


class BarTest(unittest.TestCase):
    def test_read_datafile(self):
        datafile = os.environ.get("DATAFILE")
        self.assertIsNotNone(datafile, "DATAFILE env var should be set by runner")
        with open(datafile, "r", encoding="utf-8") as f:
            content = f.read().strip()
        self.assertEqual(content, "Hello from tools/myrununder/runner.txt!")


if __name__ == "__main__":
    unittest.main()
