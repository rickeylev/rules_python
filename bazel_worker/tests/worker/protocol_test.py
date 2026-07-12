import io
import unittest
from bazel_tools.src.main.protobuf.worker_protocol_pb2 import (
    Input,
    WorkRequest,
    WorkResponse,
)
from bazel_worker.protocol import (
    read_request_from_stream,
    read_response_from_stream,
    write_request_to_stream,
    write_response_to_stream,
)


class ProtocolTest(unittest.TestCase):

    def test_json_work_request_roundtrip(self) -> None:
        req = WorkRequest()
        req.arguments.extend(["--foo", "bar"])
        inp = req.inputs.add()
        inp.path = "src/foo.py"
        inp.digest = b"1234"
        req.request_id = 42
        req.cancel = False
        req.verbosity = 1
        req.sandbox_dir = "/tmp/sandbox"

        stream = io.StringIO()
        write_request_to_stream(req, "json", stream)
        stream.seek(0)
        decoded = read_request_from_stream(stream, "json")
        self.assertIsNotNone(decoded)
        assert decoded is not None
        self.assertEqual(list(decoded.arguments), ["--foo", "bar"])
        self.assertEqual(len(decoded.inputs), 1)
        self.assertEqual(decoded.inputs[0].path, "src/foo.py")
        self.assertEqual(decoded.request_id, 42)
        self.assertFalse(decoded.cancel)
        self.assertEqual(decoded.verbosity, 1)
        self.assertEqual(decoded.sandbox_dir, "/tmp/sandbox")

    def test_json_work_response_roundtrip(self) -> None:
        resp = WorkResponse()
        resp.exit_code = 0
        resp.output = "build success"
        resp.request_id = 42
        resp.was_cancelled = False

        stream = io.StringIO()
        write_response_to_stream(resp, "json", stream)
        stream.seek(0)
        decoded = read_response_from_stream(stream, "json")
        self.assertIsNotNone(decoded)
        assert decoded is not None
        self.assertEqual(decoded.exit_code, 0)
        self.assertEqual(decoded.output, "build success")
        self.assertEqual(decoded.request_id, 42)
        self.assertFalse(decoded.was_cancelled)

    def test_proto_work_request_roundtrip(self) -> None:
        req = WorkRequest()
        req.arguments.extend(["--foo", "bar"])
        inp = req.inputs.add()
        inp.path = "src/foo.py"
        inp.digest = b"1234"
        req.request_id = 42
        req.cancel = False
        req.verbosity = 1
        req.sandbox_dir = "/tmp/sandbox"

        stream = io.BytesIO()
        write_request_to_stream(req, "proto", stream)
        stream.seek(0)
        decoded = read_request_from_stream(stream, "proto")
        self.assertIsNotNone(decoded)
        assert decoded is not None
        self.assertEqual(list(decoded.arguments), ["--foo", "bar"])
        self.assertEqual(len(decoded.inputs), 1)
        self.assertEqual(decoded.inputs[0].path, "src/foo.py")
        self.assertEqual(decoded.request_id, 42)
        self.assertFalse(decoded.cancel)
        self.assertEqual(decoded.verbosity, 1)
        self.assertEqual(decoded.sandbox_dir, "/tmp/sandbox")


if __name__ == "__main__":
    unittest.main()
