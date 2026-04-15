import sys
import unittest

class SharedLibLoadingWindowsTest(unittest.TestCase):
    def setUp(self):
        super().setUp()
        if sys.prefix == sys.base_prefix:
            raise AssertionError("Not running under a venv")
        self.venv = sys.prefix

    def test_shared_library_loading(self):
        # The TODO says to import win32.timer.
        # pywin32 modules are often accessible via the win32 package.
        try:
            import win32.timer
            module = win32.timer
        except ImportError:
            # Fallback to win32timer if win32.timer is not available.
            # Depending on how pywin32 is packaged/installed, it might be top-level.
            import win32timer
            module = win32timer

        self.assertIsNotNone(module.__file__)
        
        # Verify it's in the venv
        # Normalize paths for Windows comparison
        actual_file = module.__file__.lower().replace("\\", "/")
        expected_prefix = self.venv.lower().replace("\\", "/")
        
        self.assertTrue(
            actual_file.startswith(expected_prefix),
            f"Module {module.__name__} not loaded from venv.\n"
            f"Venv: {self.venv}\n"
            f"Module file: {module.__file__}"
        )
        
        # Verify it's a shared library (.pyd)
        self.assertTrue(
            actual_file.endswith(".pyd"),
            f"Expected .pyd extension, got {module.__file__}"
        )

        # Verify it works
        self.assertTrue(hasattr(module, 'set_timer'))

if __name__ == "__main__":
    unittest.main()
