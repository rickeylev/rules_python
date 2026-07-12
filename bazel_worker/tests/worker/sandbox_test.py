import unittest
from bazel_worker_testing.sandbox import WorkerSandbox


class SandboxTest(unittest.TestCase):

    def test_write_and_read_file(self) -> None:
        with WorkerSandbox() as sandbox:
            inp = sandbox.write_file("pkg/test.txt", "hello world")
            self.assertEqual(inp.path, "pkg/test.txt")
            self.assertTrue(sandbox.exists("pkg/test.txt"))
            self.assertEqual(
                (sandbox.path / "pkg/test.txt").read_text("utf-8"),
                "hello world",
            )

    def test_write_file_changes_digest(self) -> None:
        with WorkerSandbox() as sandbox:
            inp1 = sandbox.write_file("pkg/test.txt", "v1")
            inp2 = sandbox.write_file("pkg/test.txt", "v2")
            self.assertEqual(inp1.path, inp2.path)
            self.assertNotEqual(inp1.digest, inp2.digest)
            self.assertEqual(
                (sandbox.path / "pkg/test.txt").read_text("utf-8"), "v2"
            )


if __name__ == "__main__":
    unittest.main()
