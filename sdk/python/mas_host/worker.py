"""Serve one generated Python worker invocation over its inherited private socket."""
from __future__ import annotations

import importlib
import os
import socket
from collections.abc import Callable
from typing import Any

from .adaptation import adapt
from .codec import validate_object
from .context import Context
from .errors import EffectUncertain, HostError
from .transport import FramedChannel

CONTRACT_VERSION = "mas-native/1"


def _channel() -> FramedChannel:
    try:
        descriptor = int(os.environ["MAS_WORKER_CHANNEL_FD"])
    except (KeyError, ValueError) as error:
        raise HostError("MAS_WORKER_CHANNEL_FD must identify a private socket") from error
    if descriptor < 3:
        raise HostError("private worker channel cannot use stdin/stdout/stderr")
    # Claim the original descriptor and revoke inheritance before importing any
    # application module or allowing its code to launch subprocesses.
    connection = socket.socket(fileno=descriptor)
    connection.set_inheritable(False)
    return FramedChannel(connection)


def _dispatch(channel: FramedChannel, expected_handler: str,
              load_handler: Callable[[], Callable[..., Any]]) -> None:
    context = None
    try:
        start = channel.receive()
        if start.get("type") != "start" or start.get("version") != CONTRACT_VERSION:
            raise HostError("invalid native start")
        if start.get("handler") != expected_handler:
            channel.send({"type": "error", "message": "unknown native handler"})
            return
        actor, invocation = start.get("actor"), start.get("invocation_id")
        deadline = start.get("deadline_ms")
        if (type(actor) is not str or not actor or type(invocation) is not str or not invocation
                or type(deadline) is not int or deadline < 0):
            raise HostError("native start requires actor, invocation identity and original deadline")
        validate_object(start.get("input"))
        validate_object(start.get("configuration", {}))
        channel = FramedChannel(channel.connection, deadline)
        context = Context(channel, actor, invocation, deadline)
        try:
            result = adapt(load_handler())(context, start["input"])
            if context._uncertain:
                raise EffectUncertain("admitted effect requires reconciliation")
            channel.send({"type": "complete", "output": result})
        except Exception as error:
            # Bound application diagnostics so their frame cannot exceed the
            # native limit or contain invalid Unicode from an exception string.
            message = str(error).encode("utf-8", "replace").decode("utf-8")[:2048]
            channel.send({"type": "error", "message": message or type(error).__name__})
    finally:
        if context is not None:
            context._active = False
        channel.close()


def serve(handler_name: str, handler: Callable[..., Any]) -> None:
    """Generated entry point for an already imported synchronous handler."""
    if type(handler_name) is not str or not handler_name:
        raise ValueError("handler_name must be a nonempty string")
    _dispatch(_channel(), handler_name, lambda: handler)


def serve_module(handler_name: str, module_name: str) -> None:
    """Adopt the worker socket before importing the frozen application module."""
    if type(handler_name) is not str or not handler_name:
        raise ValueError("handler_name must be a nonempty string")
    if type(module_name) is not str or not module_name:
        raise ValueError("module_name must be a nonempty string")

    def load() -> Callable[..., Any]:
        return getattr(importlib.import_module(module_name), handler_name)

    _dispatch(_channel(), handler_name, load)
