import os
import sys
import unittest
from pathlib import Path
import importlib

class SharedLibLoadingWindowsTest(unittest.TestCase):
    def setUp(self):
        super().setUp()
        if sys.prefix == sys.base_prefix:
            raise AssertionError("Not running under a venv")
        self.venv = Path(sys.prefix)

    def test_shared_library_loading(self):
        # We import markupsafe._speedups (a .cp311-win_amd64.pyd extension)
        import markupsafe._speedups
        module = markupsafe._speedups

        print(f"Module file: {module.__file__}")

        # Verify it's in the venv
        # Normalize paths for Windows comparison. 
        # We DON'T use resolve() here because we want to see the path Python used,
        # which should be within the venv's site-packages (even if it's a symlink).
        actual_file = str(Path(module.__file__)).lower()
        expected_prefix = str(self.venv).lower()

        self.assertTrue(
            actual_file.startswith(expected_prefix),
            f"Module {module.__name__} not loaded from venv.\n"
            f"Venv: {expected_prefix}\n"
            f"Module file: {actual_file}\n"
            f"sys.path:\n" + "\n".join(sys.path)
        )

        # Verify it's a shared library (.pyd)
        self.assertTrue(
            actual_file.endswith(".pyd"),
            f"Expected .pyd extension, got {module.__file__}"
        )


if __name__ == "__main__":
    unittest.main()
