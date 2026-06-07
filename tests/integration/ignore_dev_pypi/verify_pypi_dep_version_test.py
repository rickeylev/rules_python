import unittest

import click


class VerifyPypiDepVersionTest(unittest.TestCase):
    def test_click_version(self):
        self.assertEqual(click.__version__, "8.1.7")


if __name__ == "__main__":
    unittest.main()
