"""Frame private worker-socket messages under the original absolute deadline."""
from __future__ import annotations

import socket
import struct
import time
from typing import Any

from .codec import MAX_FRAME_BYTES, dumps_object, loads_object


class FramedChannel:
    """One socket; the owning Context serialises request/response exchanges."""

    def __init__(self, connection: socket.socket, deadline_ms: int = 0):
        if type(deadline_ms) is not int or deadline_ms < 0:
            raise ValueError("deadline_ms must be a nonnegative integer")
        self.connection = connection
        self.deadline_ms = deadline_ms
        self._monotonic_deadline = (
            time.monotonic() + max(0, deadline_ms / 1000 - time.time())
            if deadline_ms else None
        )

    def _deadline(self) -> None:
        if self._monotonic_deadline is None:
            return
        remaining = min(self.deadline_ms / 1000 - time.time(),
                        self._monotonic_deadline - time.monotonic())
        if remaining <= 0:
            raise TimeoutError("original worker deadline has elapsed")
        self.connection.settimeout(remaining)

    def send(self, value: Any) -> None:
        data = dumps_object(value)
        self._deadline()
        self.connection.sendall(struct.pack(">I", len(data)) + data)

    def _read(self, size: int) -> bytes:
        parts = bytearray()
        while len(parts) < size:
            self._deadline()
            chunk = self.connection.recv(size - len(parts))
            if not chunk:
                raise EOFError("private worker channel closed during frame")
            parts.extend(chunk)
        return bytes(parts)

    def receive(self) -> dict[str, Any]:
        length, = struct.unpack(">I", self._read(4))
        if not 1 <= length <= MAX_FRAME_BYTES:
            raise ValueError("native frame length must be 1..65536 bytes")
        return loads_object(self._read(length))

    def close(self) -> None:
        self.connection.close()
