"""Exchange one bounded length-prefixed frame on the guardian's inherited channel."""
import os
import struct

from .contracts import AdapterError, MAX_FRAME, require


def read_exact(fd, size):
    data = bytearray()
    while len(data) < size:
        part = os.read(fd, size - len(data))
        require(bool(part), "channel-incomplete")
        data.extend(part)
    return bytes(data)


def read_frame(fd):
    size = struct.unpack(">I", read_exact(fd, 4))[0]
    require(0 < size <= MAX_FRAME, "frame-limit")
    return read_exact(fd, size)


def write_frame(fd, data):
    require(0 < len(data) <= MAX_FRAME, "frame-limit")
    view = memoryview(struct.pack(">I", len(data)) + data)
    while view:
        count = os.write(fd, view)
        if count <= 0:
            raise AdapterError("channel-incomplete")
        view = view[count:]
