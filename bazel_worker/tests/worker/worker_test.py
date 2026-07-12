import json
import sys
import unittest
from pathlib import Path
from bazel_worker import requirements
from bazel_worker_testing import worker as worker_testing


def simple_echo_worker() -> None:
    """A minimal persistent worker loop that reads requests from stdin and responds."""
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        req = json.loads(line)
        request_id = req.get("requestId", 0)
        args = req.get("arguments", [])
        if "--fail" in args:
            resp = {"exitCode": 1, "output": "failed", "requestId": request_id}
        else:
            for i, arg in enumerate(args):
                if arg == "-o" and i + 1 < len(args):
                    with open(args[i + 1], "w", encoding="utf-8") as f:
                        f.write("compiled")
            resp = {"exitCode": 0, "output": "success", "requestId": request_id}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


def multiplex_echo_worker() -> None:
    """A multiplex persistent worker loop that reads sandbox_dir and request inputs."""
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        req = json.loads(line)
        request_id = req.get("requestId", 0)
        sandbox_dir = req.get("sandboxDir")
        args = req.get("arguments", [])
        output_msg = f"multiplex_ack_{request_id}"
        if sandbox_dir and args and args[0] == "--check-file":
            file_path = Path(sandbox_dir) / args[1]
            if file_path.exists():
                output_msg = file_path.read_text("utf-8")
        resp = {"exitCode": 0, "output": output_msg, "requestId": request_id}
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


class FrameworkTest(unittest.TestCase):

    JSON_REQS = requirements.REQUIRES_JSON

    def test_start_worker_and_send_request(self) -> None:
        with worker_testing.start_worker(simple_echo_worker, self.JSON_REQS) as worker:
            req = worker.create_request()
            req.args = ["--foo", "bar"]
            req.request_id = 1
            resp = worker.send_request(req)
            self.assertEqual(resp.exit_code, 0)
            self.assertEqual(resp.output, "success")
            self.assertEqual(resp.request_id, 1)

    def test_request_builder_singleplex(self) -> None:
        with worker_testing.start_worker(simple_echo_worker, self.JSON_REQS) as worker:
            req = worker.create_request()
            req.args = ["-o", "out.txt"]
            req.add_input("in.txt", content="input text")
            resp = worker.send_request(req)
            self.assertEqual(resp.exit_code, 0)
            self.assertTrue(worker.sandbox.exists("out.txt"))
            self.assertTrue(worker.sandbox.exists("in.txt"))
            self.assertEqual(
                (worker.sandbox.path / "in.txt").read_text("utf-8"),
                "input text",
            )

    def test_request_builder_multiplex(self) -> None:
        reqs = {
            **requirements.SUPPORTS_WORKERS,
            **requirements.SUPPORTS_MULTIPLEX_WORKERS,
            **requirements.REQUIRES_JSON,
        }
        with worker_testing.start_worker(multiplex_echo_worker, reqs) as worker:
            self.assertTrue(worker.is_multiplex)

            req1 = worker.create_request()
            req1.args = ["--check-file", "foo/bar.txt"]
            req1.add_input("foo/bar.txt", content="multiplex content 1")
            resp1 = worker.send_request(req1)
            self.assertEqual(resp1.output, "multiplex content 1")
            self.assertEqual(resp1.request_id, 1)
            self.assertTrue(
                (worker.sandbox.path / "multiplex_1" / "foo/bar.txt").exists()
            )

            req2 = worker.create_request()
            req2.args = ["--check-file", "foo/bar.txt"]
            req2.add_input("foo/bar.txt", content="multiplex content 2")
            resp2 = worker.send_request(req2)
            self.assertEqual(resp2.output, "multiplex content 2")
            self.assertEqual(resp2.request_id, 2)
            self.assertTrue(
                (worker.sandbox.path / "multiplex_2" / "foo/bar.txt").exists()
            )

    def test_worker_error_response(self) -> None:
        with worker_testing.start_worker(simple_echo_worker, self.JSON_REQS) as worker:
            req = worker.create_request()
            req.args = ["--fail"]
            req.request_id = 3
            resp = worker.send_request(req)
            self.assertEqual(resp.exit_code, 1)
            self.assertEqual(resp.output, "failed")

    def test_start_twice_raises(self) -> None:
        with worker_testing.RunningWorker.create(simple_echo_worker, self.JSON_REQS) as worker:
            with self.assertRaises(RuntimeError):
                worker.start()


if __name__ == "__main__":
    unittest.main()
