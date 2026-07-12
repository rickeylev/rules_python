import hashlib
import shutil
import tempfile
from pathlib import Path
from typing import Any, Union
from bazel_tools.src.main.protobuf.worker_protocol_pb2 import Input


class WorkerSandbox:
    """Manages a simulated execution root / sandbox directory for worker testing."""

    def __init__(self, prefix: str = "worker_sandbox_") -> None:
        self._tempdir = tempfile.mkdtemp(prefix=prefix)
        self.path: Path = Path(self._tempdir).resolve()

    def write_file(
        self, relative_path: Union[str, Path], content: Union[str, bytes]
    ) -> Input:
        """Writes a file to the sandbox and returns a protobuf Input with digest."""
        full_path = self.path / relative_path
        full_path.parent.mkdir(parents=True, exist_ok=True)
        data = content.encode("utf-8") if isinstance(content, str) else content
        full_path.write_bytes(data)
        digest = hashlib.sha256(data).digest()
        inp = Input()
        inp.path = str(relative_path)
        inp.digest = digest
        return inp

    def exists(self, relative_path: Union[str, Path]) -> bool:
        """Checks whether a file or directory exists in the sandbox."""
        return (self.path / relative_path).exists()

    def close(self) -> None:
        """Removes the temporary sandbox directory and all its contents."""
        if self.path.exists():
            shutil.rmtree(self.path, ignore_errors=True)

    def __enter__(self) -> "WorkerSandbox":
        return self

    def __exit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        self.close()
