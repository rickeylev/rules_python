import json
from typing import Any, Dict, Optional
from bazel_tools.src.main.protobuf.worker_protocol_pb2 import (
    Input,
    WorkRequest,
    WorkResponse,
)


_VARINT_PAYLOAD_MASK = 0x7F
_VARINT_CONTINUATION_BIT = 0x80
_VARINT_MAX_BITS = 35
_VARINT_SHIFT = 7


def json_dict_from_input(inp: Input) -> Dict[str, Any]:
    """Serializes a protobuf Input message to a JSON protocol dictionary."""
    digest_str = (
        inp.digest.hex() if isinstance(inp.digest, bytes) else str(inp.digest)
    )
    return {"path": inp.path, "digest": digest_str}


def input_from_json_dict(data: Dict[str, Any]) -> Input:
    """Deserializes a protobuf Input message from a JSON protocol dictionary."""
    inp = Input()
    inp.path = str(data.get("path", ""))
    digest_raw = data.get("digest", "")
    if isinstance(digest_raw, str):
        try:
            inp.digest = bytes.fromhex(digest_raw)
        except ValueError:
            inp.digest = digest_raw.encode("utf-8")
    else:
        inp.digest = bytes(digest_raw)
    return inp


def json_dict_from_request(req: WorkRequest) -> Dict[str, Any]:
    """Serializes a protobuf WorkRequest message to a JSON protocol dictionary."""
    d: Dict[str, Any] = {
        "arguments": list(req.arguments),
        "inputs": [json_dict_from_input(inp) for inp in req.inputs],
        "requestId": req.request_id,
        "cancel": req.cancel,
        "verbosity": req.verbosity,
    }
    if req.sandbox_dir:
        d["sandboxDir"] = req.sandbox_dir
    return d


def request_from_json_dict(data: Dict[str, Any]) -> WorkRequest:
    """Deserializes a protobuf WorkRequest message from a JSON protocol dictionary."""
    req = WorkRequest()
    if "arguments" in data:
        req.arguments.extend([str(a) for a in data["arguments"]])
    if "inputs" in data:
        for inp_data in data["inputs"]:
            pb_inp = req.inputs.add()
            pb_inp.CopyFrom(input_from_json_dict(inp_data))
    if "requestId" in data:
        req.request_id = int(data["requestId"])
    if "cancel" in data:
        req.cancel = bool(data["cancel"])
    if "verbosity" in data:
        req.verbosity = int(data["verbosity"])
    if "sandboxDir" in data and data["sandboxDir"] is not None:
        req.sandbox_dir = str(data["sandboxDir"])
    return req


def json_dict_from_response(resp: WorkResponse) -> Dict[str, Any]:
    """Serializes a protobuf WorkResponse message to a JSON protocol dictionary."""
    return {
        "exitCode": resp.exit_code,
        "output": resp.output,
        "requestId": resp.request_id,
        "wasCancelled": resp.was_cancelled,
    }


def response_from_json_dict(data: Dict[str, Any]) -> WorkResponse:
    """Deserializes a protobuf WorkResponse message from a JSON protocol dictionary."""
    resp = WorkResponse()
    if "exitCode" in data:
        resp.exit_code = int(data["exitCode"])
    if "output" in data:
        resp.output = str(data["output"])
    if "requestId" in data:
        resp.request_id = int(data["requestId"])
    if "wasCancelled" in data:
        resp.was_cancelled = bool(data["wasCancelled"])
    return resp


def _read_varint32(stream: Any) -> Optional[int]:
    """Reads a varint32 from a binary stream for protobuf framing."""
    result = 0
    shift = 0
    while True:
        byte_raw = stream.read(1)
        if not byte_raw:
            if shift == 0:
                return None
            raise EOFError("Unexpected EOF while reading varint")
        byte = byte_raw[0] if isinstance(byte_raw, bytes) else ord(byte_raw)
        result |= (byte & _VARINT_PAYLOAD_MASK) << shift
        if not (byte & _VARINT_CONTINUATION_BIT):
            return result
        shift += _VARINT_SHIFT
        if shift >= _VARINT_MAX_BITS:
            raise ValueError("Varint too long")


def _write_varint32(stream: Any, value: int) -> None:
    """Writes a varint32 prefix to a binary stream for protobuf framing."""
    while value > _VARINT_PAYLOAD_MASK:
        b = (value & _VARINT_PAYLOAD_MASK) | _VARINT_CONTINUATION_BIT
        stream.write(
            bytes([b])
            if hasattr(stream, "readinto") or isinstance(stream.read(0), bytes)
            else chr(b)
        )
        value >>= _VARINT_SHIFT
    stream.write(
        bytes([value & _VARINT_PAYLOAD_MASK])
        if hasattr(stream, "readinto") or isinstance(stream.read(0), bytes)
        else chr(value & _VARINT_PAYLOAD_MASK)
    )


def write_json_request_to_stream(request: WorkRequest, stream: Any) -> None:
    """Writes a JSON-formatted WorkRequest to the stream."""
    data = json.dumps(json_dict_from_request(request)) + "\n"
    stream.write(data)
    stream.flush()


def write_proto_request_to_stream(request: WorkRequest, stream: Any) -> None:
    """Writes a protobuf-formatted WorkRequest to the stream."""
    serialized = request.SerializeToString()
    _write_varint32(stream, len(serialized))
    stream.write(serialized)
    stream.flush()


def write_request_to_stream(
    request: WorkRequest, protocol: str, stream: Any
) -> None:
    """Writes a WorkRequest to the stream using the specified protocol."""
    protocol_lower = protocol.lower()
    if protocol_lower == "json":
        write_json_request_to_stream(request, stream)
    elif protocol_lower == "proto":
        write_proto_request_to_stream(request, stream)
    else:
        raise ValueError(f"Unsupported worker protocol: {protocol}")


def read_json_response_from_stream(stream: Any) -> Optional[WorkResponse]:
    """Reads a JSON-formatted WorkResponse from the stream."""
    line = stream.readline()
    if not line:
        return None
    return response_from_json_dict(json.loads(line))


def read_proto_response_from_stream(stream: Any) -> Optional[WorkResponse]:
    """Reads a protobuf-formatted WorkResponse from the stream."""
    length = _read_varint32(stream)
    if length is None:
        return None
    data = stream.read(length)
    if len(data) < length:
        raise EOFError("Unexpected EOF while reading protobuf payload")
    msg = WorkResponse()
    msg.ParseFromString(data)
    return msg


def read_response_from_stream(
    stream: Any, protocol: str
) -> Optional[WorkResponse]:
    """Reads a WorkResponse from the stream using the specified protocol."""
    protocol_lower = protocol.lower()
    if protocol_lower == "json":
        return read_json_response_from_stream(stream)
    elif protocol_lower == "proto":
        return read_proto_response_from_stream(stream)
    else:
        raise ValueError(f"Unsupported worker protocol: {protocol}")


def read_json_request_from_stream(stream: Any) -> Optional[WorkRequest]:
    """Reads a JSON-formatted WorkRequest from the stream."""
    line = stream.readline()
    if not line:
        return None
    return request_from_json_dict(json.loads(line))


def read_proto_request_from_stream(stream: Any) -> Optional[WorkRequest]:
    """Reads a protobuf-formatted WorkRequest from the stream."""
    length = _read_varint32(stream)
    if length is None:
        return None
    data = stream.read(length)
    if len(data) < length:
        raise EOFError("Unexpected EOF while reading protobuf payload")
    msg = WorkRequest()
    msg.ParseFromString(data)
    return msg


def read_request_from_stream(
    stream: Any, protocol: str
) -> Optional[WorkRequest]:
    """Reads a WorkRequest from the stream using the specified protocol."""
    protocol_lower = protocol.lower()
    if protocol_lower == "json":
        return read_json_request_from_stream(stream)
    elif protocol_lower == "proto":
        return read_proto_request_from_stream(stream)
    else:
        raise ValueError(f"Unsupported worker protocol: {protocol}")


def write_json_response_to_stream(response: WorkResponse, stream: Any) -> None:
    """Writes a JSON-formatted WorkResponse to the stream."""
    data = json.dumps(json_dict_from_response(response)) + "\n"
    stream.write(data)
    stream.flush()


def write_proto_response_to_stream(response: WorkResponse, stream: Any) -> None:
    """Writes a protobuf-formatted WorkResponse to the stream."""
    serialized = response.SerializeToString()
    _write_varint32(stream, len(serialized))
    stream.write(serialized)
    stream.flush()


def write_response_to_stream(
    response: WorkResponse, protocol: str, stream: Any
) -> None:
    """Writes a WorkResponse to the stream using the specified protocol."""
    protocol_lower = protocol.lower()
    if protocol_lower == "json":
        write_json_response_to_stream(response, stream)
    elif protocol_lower == "proto":
        write_proto_response_to_stream(response, stream)
    else:
        raise ValueError(f"Unsupported worker protocol: {protocol}")
