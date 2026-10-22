"""Supply complete synthetic neural responses through the real offline channel."""
import json
import os
from pathlib import Path
import socket
import struct


channel = socket.socket(fileno=int(os.environ["MAS_WORKER_CHANNEL_FD"]))


def read(size):
    value = bytearray()
    while len(value) < size:
        part = channel.recv(size - len(value))
        if not part:
            raise RuntimeError("synthetic-inspection-frame-unavailable")
        value += part
    return bytes(value)


length = struct.unpack("!I", read(4))[0]
if not 1 <= length <= 65536:
    raise RuntimeError("synthetic-inspection-frame-limit")
request = json.loads(read(length))
with open("inspection-calls", "a", encoding="ascii") as stream:
    stream.write("x")
if request["text"] == "governed-acquisition-original-person@example.invalid":
    Path("original-inspected").write_text("seen", encoding="ascii")
response = {"version": "mas-privacy-worker-response/1", "source_id": request["source_id"],
            "source_token": request["source_token"], "input_bytes": request["input_bytes"],
            "detector": request["detector"], "offset_unit": "unicode-codepoints",
            "completion": "complete", "checked_ranges": [{"start": 0, "end": len(request["text"])}]
            if request["text"] else [], "omissions": [], "findings": []}
raw = json.dumps(response, separators=(",", ":")).encode()
channel.sendall(struct.pack("!I", len(raw)) + raw)
