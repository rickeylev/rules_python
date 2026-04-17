import unittest
import zipfile
import tempfile
import pathlib
import os

from tools.private.stdlib_zipper import stdlib_zipper

class TestStdlibZipper(unittest.TestCase):
    def test_create_zip(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            temp_path = pathlib.Path(temp_dir)
            f1 = temp_path / "b.py"
            f1.write_text("print('b')")
            f2 = temp_path / "a.py"
            f2.write_text("print('a')")
            
            manifest_path = temp_path / "manifest.txt"
            manifest_path.write_text(f"b.py|{f1}\na.py|{f2}\n")
            
            zip_path = temp_path / "out.zip"
            
            stdlib_zipper.main([
                "--out", str(zip_path),
                "--strip-prefix", "my/prefix",
                "--manifest", str(manifest_path)
            ])
            
            self.assertTrue(zip_path.exists())
            
            with zipfile.ZipFile(zip_path, 'r') as zf:
                infos = zf.infolist()
                
                # Check deterministic sorting
                self.assertEqual(len(infos), 2)
                self.assertEqual(infos[0].filename, "a.py")
                self.assertEqual(infos[1].filename, "b.py")
                
                # Check deterministic timestamps
                self.assertEqual(infos[0].date_time, (1980, 1, 1, 0, 0, 0))
                self.assertEqual(infos[1].date_time, (1980, 1, 1, 0, 0, 0))
                
                # Check content
                self.assertEqual(zf.read("a.py").decode(), "print('a')")
                self.assertEqual(zf.read("b.py").decode(), "print('b')")

if __name__ == "__main__":
    unittest.main()
