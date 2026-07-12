import hashlib
import os
import sys
import threading
from contextlib import contextmanager
from pathlib import PurePath
from typing import Any, Callable, Dict, Iterator, List, Optional, Tuple, Union
from bazel_tools.src.main.protobuf.worker_protocol_pb2 import (
    Input,
    WorkRequest,
    WorkResponse,
)
from bazel_worker.protocol import (
    read_response_from_stream,
    write_request_to_stream,
)
from bazel_worker import requirements
from bazel_worker_testing.sandbox import WorkerSandbox


class RequestBuilder:
    """Helper for building WorkRequests and staging input files for worker tests.

    See https://github.com/bazelbuild/bazel/blob/master/src/main/protobuf/worker_protocol.proto
    for the WorkRequest proto definition.
    """

    def __init__(self) -> None:
        self.args: List[str] = []
        self._inputs: List[Tuple[str, Union[str, bytes]]] = []
        self.request_id: Optional[int] = None
        self.cancel: bool = False
        self.verbosity: int = 0

    def add_input(
        self,
        path: Union[str, PurePath],
        *,
        content: Optional[Union[str, bytes]] = None,
        content_from: Optional[Union[str, PurePath]] = None,
    ) -> "RequestBuilder":
        """Adds an input file to be staged in the sandbox.

        Args:
            path: The relative path of the file in the sandbox.
            content: The content of the file as string or bytes.
            content_from: A path to a file to read the content from.

        Returns:
            This builder instance.
        """
        if content is not None and content_from is not None:
            raise ValueError(
                "Specify at most one of `content` and `content_from`."
            )
        if content is None and content_from is None:
            raise ValueError("Must specify either `content` or `content_from`.")

        if content_from is not None:
            data: Union[str, bytes] = PurePath(content_from).read_bytes()
        else:
            assert content is not None
            data = content

        self._inputs.append((str(path), data))
        return self


class RunningWorker:
    """Manages an in-process persistent worker thread and its pipes/sandbox."""

    def __init__(
        self,
        worker_fn: Callable[..., Any],
        execution_requirements: Dict[str, str],
        worker_stdin: Any,
        worker_stdout: Any,
        host_stdin_writer: Any,
        host_stdout_reader: Any,
        sandbox: WorkerSandbox,
        *args: Any,
        **kwargs: Any,
    ) -> None:
        self.worker_fn = worker_fn
        self.execution_requirements = execution_requirements
        self.args = args
        self.kwargs = kwargs
        # Bazel defaults to "proto" if not specified.
        # See https://bazel.build/remote/creating
        self.protocol = execution_requirements.get(
            "requires-worker-protocol", "proto"
        )
        self.sandbox = sandbox
        self._worker_stdin = worker_stdin
        self._worker_stdout = worker_stdout
        self._host_stdin_writer = host_stdin_writer
        self._host_stdout_reader = host_stdout_reader
        self._worker_exception: Optional[BaseException] = None
        self._thread: Optional[threading.Thread] = None
        self._next_request_id: int = 0

    @property
    def is_multiplex(self) -> bool:
        """Returns whether this worker supports multiplex requests."""
        return (
            self.execution_requirements.get("supports-multiplex-workers") == "1"
        )

    def create_request(self) -> RequestBuilder:
        """Returns a new RequestBuilder."""
        return RequestBuilder()

    @classmethod
    @contextmanager
    def create(
        cls,
        worker_fn: Callable[..., Any],
        execution_requirements: Optional[Dict[str, str]] = None,
        *args: Any,
        **kwargs: Any,
    ) -> Iterator["RunningWorker"]:
        """Creates pipes and a sandbox, starts the worker in-process, and manages cleanup."""
        if execution_requirements is None:
            execution_requirements = {
                **requirements.SUPPORTS_WORKERS,
                **requirements.REQUIRES_PROTO,
            }
        sandbox = WorkerSandbox()
        worker_stdin = None
        worker_stdout = None
        host_stdin_writer = None
        host_stdout_reader = None
        worker = None
        try:
            _stdin_r_fd, _stdin_w_fd = os.pipe()
            _stdout_r_fd, _stdout_w_fd = os.pipe()

            worker_stdin = open(
                _stdin_r_fd, "r", encoding="utf-8", closefd=True
            )
            worker_stdout = open(
                _stdout_w_fd, "w", encoding="utf-8", closefd=True
            )
            host_stdin_writer = open(
                _stdin_w_fd, "w", encoding="utf-8", closefd=True
            )
            host_stdout_reader = open(
                _stdout_r_fd, "r", encoding="utf-8", closefd=True
            )

            worker = cls(
                worker_fn,
                execution_requirements,
                worker_stdin,
                worker_stdout,
                host_stdin_writer,
                host_stdout_reader,
                sandbox,
                *args,
                **kwargs,
            )
            worker.start()
            yield worker
        finally:
            if worker is not None:
                try:
                    worker.stop()
                except Exception:
                    pass
            for pipe in (
                host_stdin_writer,
                host_stdout_reader,
                worker_stdin,
                worker_stdout,
            ):
                if pipe is not None:
                    try:
                        pipe.close()
                    except Exception:
                        pass
            try:
                sandbox.close()
            except Exception:
                pass

    def _worker_runner(self) -> None:
        orig_stdin = sys.stdin
        orig_stdout = sys.stdout
        orig_cwd = os.getcwd()
        try:
            sys.stdin = self._worker_stdin
            sys.stdout = self._worker_stdout
            os.chdir(str(self.sandbox.path))
            self.worker_fn(*self.args, **self.kwargs)
        except BaseException as e:
            if not isinstance(e, SystemExit) or (
                isinstance(e, SystemExit)
                and e.code != 0
                and e.code is not None
            ):
                self._worker_exception = e
        finally:
            sys.stdin = orig_stdin
            sys.stdout = orig_stdout
            try:
                os.chdir(orig_cwd)
            except Exception:
                pass
            try:
                self._worker_stdout.close()
            except Exception:
                pass
            try:
                self._worker_stdin.close()
            except Exception:
                pass

    def start(self) -> "RunningWorker":
        """Starts the background worker thread redirecting sys.stdin/stdout and cwd."""
        if self._thread is not None:
            raise RuntimeError("Worker has already been started.")

        thread_name = (
            f"RunningWorkerThread-{getattr(self.worker_fn, '__name__', 'anonymous')}"
        )
        self._thread = threading.Thread(
            target=self._worker_runner, name=thread_name, daemon=True
        )
        self._thread.start()
        return self

    def send_request(
        self,
        request: Union[RequestBuilder, WorkRequest],
    ) -> WorkResponse:
        """Sends a WorkRequest and synchronously reads the matching WorkResponse."""
        if self._worker_exception is not None:
            raise self._worker_exception

        if isinstance(request, RequestBuilder):
            req = WorkRequest()
            req.arguments.extend(request.args)
            req.cancel = request.cancel
            req.verbosity = request.verbosity

            if request.request_id is not None:
                req_id = request.request_id
            else:
                self._next_request_id += 1
                req_id = self._next_request_id if self.is_multiplex else 0
            req.request_id = req_id

            if self.is_multiplex:
                sandbox_dir = f"multiplex_{req_id}"
                target_dir = self.sandbox.path / sandbox_dir
                req.sandbox_dir = sandbox_dir
            else:
                target_dir = self.sandbox.path

            for rel_path, content in request._inputs:
                full_path = target_dir / rel_path
                full_path.parent.mkdir(parents=True, exist_ok=True)
                data = (
                    content.encode("utf-8")
                    if isinstance(content, str)
                    else content
                )
                full_path.write_bytes(data)
                digest = hashlib.sha256(data).digest()
                inp = req.inputs.add()
                inp.path = str(rel_path)
                inp.digest = digest
        else:
            req = request

        orig_stdin = sys.stdin
        orig_stdout = sys.stdout
        orig_cwd = os.getcwd()
        try:
            sys.stdin = self._worker_stdin
            sys.stdout = self._worker_stdout
            os.chdir(str(self.sandbox.path))

            stream_out = (
                self._host_stdin_writer.buffer
                if self.protocol.lower() == "proto"
                else self._host_stdin_writer
            )
            write_request_to_stream(req, self.protocol, stream_out)

            stream_in = (
                self._host_stdout_reader.buffer
                if self.protocol.lower() == "proto"
                else self._host_stdout_reader
            )
            resp = read_response_from_stream(stream_in, self.protocol)
            if resp is None:
                if self._worker_exception is not None:
                    raise self._worker_exception
                raise RuntimeError(
                    "Worker stream closed unexpectedly without returning a WorkResponse."
                )
            return resp
        finally:
            sys.stdin = orig_stdin
            sys.stdout = orig_stdout
            try:
                os.chdir(orig_cwd)
            except Exception:
                pass

    def stop(self) -> None:
        """Stops the worker by closing stdin and cleaning up the sandbox and pipes."""
        try:
            self._host_stdin_writer.close()
        except Exception:
            pass
        if self._thread is not None:
            self._thread.join(timeout=30)
            if self._worker_exception is not None:
                exc = self._worker_exception
                self._worker_exception = None
                raise exc
        try:
            self._host_stdout_reader.close()
        except Exception:
            pass
        self.sandbox.close()

    def __enter__(self) -> "RunningWorker":
        if self._thread is None:
            self.start()
        return self

    def __exit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        self.stop()


def start_worker(
    worker_fn: Callable[..., Any],
    execution_requirements: Optional[Dict[str, str]] = None,
    *args: Any,
    **kwargs: Any,
) -> Iterator[RunningWorker]:
    """Returns a context manager that starts and yields a RunningWorker instance."""
    return RunningWorker.create(
        worker_fn, execution_requirements, *args, **kwargs
    )
