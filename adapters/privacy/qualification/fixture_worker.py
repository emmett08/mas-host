"""Synthetic full-coverage guardian response for qualification harness conformance.

No neural inference occurs. The real host adds its deterministic recognisers.
"""
import json
import os
import socket
import struct


def main():
    channel = socket.socket(fileno=int(os.environ["MAS_WORKER_CHANNEL_FD"]))
    def read_exact(count):
        result = bytearray()
        while len(result) < count:
            part = channel.recv(count - len(result))
            if not part:
                raise ValueError("incomplete-frame")
            result.extend(part)
        return bytes(result)
    size = struct.unpack("!I", read_exact(4))[0]
    if size > 65536:
        raise ValueError("frame-limit")
    request = json.loads(read_exact(size))
    result = {"version": "mas-privacy-worker-response/1", "source_id": request["source_id"],
        "source_token": request["source_token"], "input_bytes": request["input_bytes"],
        "detector": request["detector"], "offset_unit": "unicode-codepoints", "completion": "complete",
        "checked_ranges": [{"start": 0, "end": len(request["text"])}] if request["text"] else [],
        "omissions": [], "findings": []}
    raw = json.dumps(result, separators=(",", ":")).encode()
    channel.sendall(struct.pack("!I", len(raw)) + raw)


if __name__ == "__main__":
    main()
