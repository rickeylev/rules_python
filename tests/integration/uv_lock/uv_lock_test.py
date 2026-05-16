import io
import json
import os
import sys
import unittest
from unittest.mock import patch

from tools.toml2json import toml2json


class UvLockTest(unittest.TestCase):

    def test_toml_to_json_conversion(self):
        uv_lock_path = os.environ["UV_LOCK"]
        with patch("sys.argv", ["toml2json", uv_lock_path]):
            with patch("sys.stdout", new=io.StringIO()) as mock_stdout:
                toml2json.main()

        result = json.loads(mock_stdout.getvalue())
        packages = result.get("package", [])
        self.assertTrue(len(packages) > 0)
        self.assertEqual(packages[0]["name"], "test-project")
        self.assertEqual(packages[0]["version"], "0.0.1")


if __name__ == "__main__":
    unittest.main()
