import os
import unittest
from python.runfiles import runfiles

class RelSymlinkTest(unittest.TestCase):
    def test_rel_symlink_value(self):
        r = runfiles.Create()
        # The symlink is at 'repro/my_symlink' in runfiles
        symlink_path = r.Rlocation("repro/my_symlink")
        self.assertTrue(os.path.islink(symlink_path), f"{symlink_path} is not a symlink")

        link_value = os.readlink(symlink_path)
        expected_value = "../subrepo/data.txt"
        self.assertEqual(link_value, expected_value)

if __name__ == "__main__":
    unittest.main()
