# Copyright 2024 The Bazel Authors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""A simple precompiler to generate deterministic pyc files for Bazel."""

# NOTE: Imports specific to the persistent worker should only be imported
# when a persistent worker is used. Avoiding the unnecessary imports
# saves significant startup time for non-worker invocations.
import argparse
import os
import py_compile
import shutil
import sys


def _parse_bool(val: "str | bool") -> bool:
    if isinstance(val, bool):
        return val
    val = str(val).lower()
    if val in ("true", "1", "yes"):
        return True
    if val in ("false", "0", "no"):
        return False
    raise argparse.ArgumentTypeError(f"Invalid boolean value: {val}")


def _create_parser() -> "argparse.Namespace":
    parser = argparse.ArgumentParser(fromfile_prefix_chars="@")
    parser.add_argument("--invalidation_mode", default="CHECKED_HASH")
    parser.add_argument("--optimize", type=int, default=-1)
    parser.add_argument("--python_version")

    parser.add_argument("--src", action="append", dest="srcs")
    parser.add_argument("--src_name", "--src-name", action="append", dest="src_names")
    parser.add_argument("--pyc", action="append", dest="pycs")

    parser.add_argument("--src_dir", "--src-dir", action="append", dest="src_dirs")
    parser.add_argument("--out_dir", "--out-dir", action="append", dest="out_dirs")
    parser.add_argument("--pyc_tag", "--pyc-tag", dest="pyc_tag")
    parser.add_argument(
        "--pycache",
        type=_parse_bool,
        default=True,
    )

    parser.add_argument("--persistent_worker", action="store_true")
    parser.add_argument("--log_level", default="ERROR")
    # Bazel workers use anonymous pipes for stdio, which don't support
    # overlapped I/O required by asyncio on Windows.
    parser.add_argument(
        "--worker_impl", default="serial" if sys.platform == "win32" else "async"
    )
    return parser


def _pyc_exists(root: str, rel_path: str, pyc_tag: "str | None") -> bool:
    dir_name, file_name = os.path.split(rel_path)
    stem, _ = os.path.splitext(file_name)
    if os.path.exists(os.path.join(root, dir_name, f"{stem}.pyc")):
        return True
    if pyc_tag and os.path.exists(
        os.path.join(root, dir_name, "__pycache__", f"{stem}.{pyc_tag}.pyc")
    ):
        return True
    return False


def _copy_dir(src_dir: str, out_dir: str) -> None:
    os.makedirs(out_dir, exist_ok=True)
    try:
        os.chmod(out_dir, os.stat(out_dir).st_mode | 0o700)
    except OSError:
        pass

    for root, dirs, files in os.walk(src_dir, followlinks=False):
        rel_root = os.path.relpath(root, src_dir)
        dst_root = os.path.join(out_dir, rel_root) if rel_root != "." else out_dir

        for d in list(dirs):
            src_path = os.path.join(root, d)
            dst_path = os.path.join(dst_root, d)
            if os.path.islink(src_path):
                dirs.remove(d)
                target = os.readlink(src_path)
                if not os.path.isabs(target):
                    if os.path.lexists(dst_path):
                        try:
                            os.remove(dst_path)
                        except OSError:
                            pass
                    try:
                        os.symlink(target, dst_path)
                    except OSError:
                        _copy_dir(src_path, dst_path)
                else:
                    _copy_dir(src_path, dst_path)
            else:
                os.makedirs(dst_path, exist_ok=True)
                try:
                    os.chmod(dst_path, os.stat(dst_path).st_mode | 0o700)
                except OSError:
                    pass

        for f in files:
            src_path = os.path.join(root, f)
            dst_path = os.path.join(dst_root, f)
            if os.path.lexists(dst_path):
                try:
                    os.remove(dst_path)
                except OSError:
                    pass

            if os.path.islink(src_path):
                target = os.readlink(src_path)
                if not os.path.isabs(target):
                    try:
                        os.symlink(target, dst_path)
                    except OSError:
                        shutil.copy2(src_path, dst_path)
                        try:
                            os.chmod(dst_path, os.stat(dst_path).st_mode | 0o600)
                        except OSError:
                            pass
                else:
                    shutil.copy2(src_path, dst_path)
                    try:
                        os.chmod(dst_path, os.stat(dst_path).st_mode | 0o600)
                    except OSError:
                        pass
            else:
                shutil.copy2(src_path, dst_path)
                try:
                    os.chmod(dst_path, os.stat(dst_path).st_mode | 0o600)
                except OSError:
                    pass


def _compile_dir(
    src_dir: str,
    src_name: str,
    out_dir: str,
    pyc_tag: "str | None",
    pycache: bool,
    optimize: int,
    invalidation_mode: py_compile.PycInvalidationMode,
) -> None:
    _copy_dir(src_dir, out_dir)
    for root, _, files in sorted(os.walk(src_dir)):
        for file in sorted(files):
            if not file.endswith(".py"):
                continue
            file_path = os.path.join(root, file)
            if not os.path.isfile(file_path):
                continue
            rel_path = os.path.relpath(file_path, src_dir)
            if _pyc_exists(out_dir, rel_path, pyc_tag):
                continue
            dir_name, file_name = os.path.split(rel_path)
            stem, _ = os.path.splitext(file_name)
            if pycache:
                if not pyc_tag:
                    continue
                target_pyc = os.path.join(
                    out_dir, dir_name, "__pycache__", f"{stem}.{pyc_tag}.pyc"
                )
            else:
                target_pyc = os.path.join(out_dir, dir_name, f"{stem}.pyc")
            os.makedirs(os.path.dirname(target_pyc), exist_ok=True)
            dfile = os.path.join(src_name, rel_path) if src_name else rel_path
            py_compile.compile(
                file_path,
                target_pyc,
                doraise=True,
                dfile=dfile,
                optimize=optimize,
                invalidation_mode=invalidation_mode,
            )


def _compile(options: "argparse.Namespace") -> None:
    try:
        invalidation_mode = py_compile.PycInvalidationMode[
            options.invalidation_mode.upper()
        ]
    except KeyError as e:
        raise ValueError(
            f"Unknown PycInvalidationMode: {options.invalidation_mode}"
        ) from e

    if options.srcs:
        if not (len(options.srcs) == len(options.src_names) == len(options.pycs)):
            raise AssertionError(
                "Mismatched number of --src, --src_name, and/or --pyc args"
            )

        for src, src_name, pyc in zip(options.srcs, options.src_names, options.pycs):
            py_compile.compile(
                src,
                pyc,
                doraise=True,
                dfile=src_name,
                optimize=options.optimize,
                invalidation_mode=invalidation_mode,
            )

    if options.src_dirs:
        out_dirs = options.out_dirs or []
        src_names = options.src_names or ["" for _ in options.src_dirs]
        if len(options.src_dirs) != len(out_dirs):
            raise AssertionError("Mismatched number of --src-dir and --out-dir args")
        for src_dir, src_name, out_dir in zip(options.src_dirs, src_names, out_dirs):
            _compile_dir(
                src_dir=src_dir,
                src_name=src_name,
                out_dir=out_dir,
                pyc_tag=options.pyc_tag,
                pycache=options.pycache,
                optimize=options.optimize,
                invalidation_mode=invalidation_mode,
            )

    return 0


# A stub type alias for readability.
# See the Bazel WorkRequest object definition:
# https://github.com/bazelbuild/bazel/blob/master/src/main/protobuf/worker_protocol.proto
JsonWorkRequest = object

# A stub type alias for readability.
# See the Bazel WorkResponse object definition:
# https://github.com/bazelbuild/bazel/blob/master/src/main/protobuf/worker_protocol.proto
JsonWorkResponse = object


class _SerialPersistentWorker:
    """Simple, synchronous, serial persistent worker."""

    def __init__(self, instream: "typing.TextIO", outstream: "typing.TextIO"):  # noqa: F821
        self._instream = instream
        self._outstream = outstream
        self._parser = _create_parser()

    def run(self) -> None:
        try:
            while True:
                request = None
                try:
                    request = self._get_next_request()
                    if request is None:
                        _logger.info("Empty request: exiting")
                        break
                    response = self._process_request(request)
                    if response:  # May be none for cancel request
                        self._send_response(response)
                except Exception:
                    _logger.exception("Unhandled error: request=%s", request)
                    output = (
                        f"Unhandled error:\nRequest: {request}\n"
                        + traceback.format_exc()
                    )
                    request_id = 0 if not request else request.get("requestId", 0)
                    self._send_response(
                        {
                            "exitCode": 3,
                            "output": output,
                            "requestId": request_id,
                        }
                    )
        finally:
            _logger.info("Worker shutting down")

    def _get_next_request(self) -> "object | None":
        line = self._instream.readline()
        if not line:
            return None
        return json.loads(line)

    def _process_request(self, request: "JsonWorkRequest") -> "JsonWorkResponse | None":
        if request.get("cancel"):
            return None
        options = self._options_from_request(request)
        _compile(options)
        response = {
            "requestId": request.get("requestId", 0),
            "exitCode": 0,
        }
        return response

    def _options_from_request(
        self, request: "JsonWorkResponse"
    ) -> "argparse.Namespace":
        options = self._parser.parse_args(request["arguments"])
        if request.get("sandboxDir"):
            prefix = request["sandboxDir"]
            options.srcs = [os.path.join(prefix, v) for v in options.srcs]
            options.pycs = [os.path.join(prefix, v) for v in options.pycs]
        return options

    def _send_response(self, response: "JsonWorkResponse") -> None:
        self._outstream.write(json.dumps(response) + "\n")
        self._outstream.flush()


class _AsyncPersistentWorker:
    """Asynchronous, concurrent, persistent worker."""

    def __init__(self, reader: "typing.TextIO", writer: "typing.TextIO"):  # noqa: F821
        self._reader = reader
        self._writer = writer
        self._parser = _create_parser()
        self._request_id_to_task = {}
        self._task_to_request_id = {}

    @classmethod
    async def main(cls, instream: "typing.TextIO", outstream: "typing.TextIO") -> None:  # noqa: F821
        reader, writer = await cls._connect_streams(instream, outstream)
        await cls(reader, writer).run()

    @classmethod
    async def _connect_streams(
        cls,
        instream: "typing.TextIO",  # noqa: F821
        outstream: "typing.TextIO",  # noqa: F821
    ) -> "tuple[asyncio.StreamReader, asyncio.StreamWriter]":
        loop = asyncio.get_event_loop()
        # Cap reader at 4 MiB, leaving enough headroom over the default 64 KiB
        # for request lines with numerous inputs (~470 KiB as of CPython 3.11).
        reader = asyncio.StreamReader(limit=1 << 22)
        protocol = asyncio.StreamReaderProtocol(reader)
        await loop.connect_read_pipe(lambda: protocol, instream)

        w_transport, w_protocol = await loop.connect_write_pipe(
            asyncio.streams.FlowControlMixin, outstream
        )
        writer = asyncio.StreamWriter(w_transport, w_protocol, reader, loop)
        return reader, writer

    async def run(self) -> None:
        while True:
            _logger.info("pending requests: %s", len(self._request_id_to_task))
            request = await self._get_next_request()
            request_id = request.get("requestId", 0)
            task = asyncio.create_task(
                self._process_request(request), name=f"request_{request_id}"
            )
            self._request_id_to_task[request_id] = task
            self._task_to_request_id[task] = request_id
            task.add_done_callback(self._handle_task_done)

    async def _get_next_request(self) -> "JsonWorkRequest":
        _logger.debug("awaiting line")
        line = await self._reader.readline()
        _logger.debug("recv line: %s", line)
        return json.loads(line)

    def _handle_task_done(self, task: "asyncio.Task") -> None:
        request_id = self._task_to_request_id[task]
        _logger.info("task done: %s %s", request_id, task)
        del self._task_to_request_id[task]
        del self._request_id_to_task[request_id]

    async def _process_request(self, request: "JsonWorkRequest") -> None:
        _logger.info("request %s: start: %s", request.get("requestId"), request)
        try:
            if request.get("cancel", False):
                await self._process_cancel_request(request)
            else:
                await self._process_compile_request(request)
        except asyncio.CancelledError:
            _logger.info(
                "request %s: cancel received, stopping processing",
                request.get("requestId"),
            )
            # We don't send a response because we assume the request that
            # triggered cancelling sent the response
            raise
        except Exception:
            _logger.exception("Unhandled error: request=%s", request)
            self._send_response(
                {
                    "exitCode": 3,
                    "output": f"Unhandled error:\nRequest: {request}\n"
                    + traceback.format_exc(),
                    "requestId": 0 if not request else request.get("requestId", 0),
                }
            )

    async def _process_cancel_request(self, request: "JsonWorkRequest") -> None:
        request_id = request.get("requestId", 0)
        task = self._request_id_to_task.get(request_id)
        if not task:
            # It must be already completed, so ignore the request, per spec
            return

        task.cancel()
        self._send_response({"requestId": request_id, "wasCancelled": True})

    async def _process_compile_request(self, request: "JsonWorkRequest") -> None:
        options = self._options_from_request(request)
        # _compile performs a varity of blocking IO calls, so run it separately
        await asyncio.to_thread(_compile, options)
        self._send_response(
            {
                "requestId": request.get("requestId", 0),
                "exitCode": 0,
            }
        )

    def _options_from_request(self, request: "JsonWorkRequest") -> "argparse.Namespace":
        options = self._parser.parse_args(request["arguments"])
        if request.get("sandboxDir"):
            prefix = request["sandboxDir"]
            options.srcs = [os.path.join(prefix, v) for v in options.srcs]
            options.pycs = [os.path.join(prefix, v) for v in options.pycs]
        return options

    def _send_response(self, response: "JsonWorkResponse") -> None:
        _logger.info("request %s: respond: %s", response.get("requestId"), response)
        self._writer.write(json.dumps(response).encode("utf8") + b"\n")


def main(args: "list[str]") -> int:
    options = _create_parser().parse_args(args)

    # Persistent workers are started with the `--persistent_worker` flag.
    # See the following docs for details on persistent workers:
    # https://bazel.build/remote/persistent
    # https://bazel.build/remote/multiplex
    # https://bazel.build/remote/creating
    if options.persistent_worker:
        global asyncio, itertools, json, logging, os, traceback, _logger
        import asyncio
        import itertools
        import json
        import logging
        import os.path
        import traceback

        _logger = logging.getLogger("precompiler")
        # Only configure logging for workers. This prevents non-worker
        # invocations from spamming stderr with logging info
        logging.basicConfig(level=getattr(logging, options.log_level))
        _logger.info("persistent worker: impl=%s", options.worker_impl)
        if options.worker_impl == "serial":
            _SerialPersistentWorker(sys.stdin, sys.stdout).run()
        elif options.worker_impl == "async":
            asyncio.run(_AsyncPersistentWorker.main(sys.stdin, sys.stdout))
        else:
            raise ValueError(f"Unknown worker impl: {options.worker_impl}")
    else:
        _compile(options)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
