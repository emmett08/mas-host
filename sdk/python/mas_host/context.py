"""Expose durable requests and explicitly admitted external effects to handlers."""
from __future__ import annotations

import threading
import time
import unicodedata
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from typing import Any

from .adaptation import decode, encode
from .codec import dumps_object, validate_object
from .errors import EffectUncertain, HostError
from .transport import FramedChannel


@dataclass(frozen=True)
class Request:
    key: str
    target: str
    input: Any


@dataclass(frozen=True)
class EffectResult:
    output: Any
    evidence: Any


def _key(value: str, label: str) -> None:
    if type(value) is not str or not value:
        raise ValueError(f"{label} must be a nonempty string")


def _step_key(value: str) -> None:
    _key(value, "step key")
    try:
        size = len(value.encode("utf-8"))
    except UnicodeEncodeError as error:
        raise ValueError("step key must contain valid Unicode") from error
    if size > 128 or any(unicodedata.category(char) == "Cc" for char in value):
        raise ValueError("step key must contain 1..128 UTF-8 bytes without control characters")


def _success(response: dict[str, Any]) -> None:
    if response.get("type") != "response" or response.get("ok") is not True:
        raise HostError("invalid host response")


def _completed_output(response: dict[str, Any], effect_id: str,
                      original: dict[str, Any]) -> dict[str, Any]:
    if "completion" not in response:
        return original
    completion = response["completion"]
    fields = {"version", "effect_id", "host_settlement", "external_execution",
              "payload_disposition", "output"}
    if (type(completion) is not dict or set(completion) != fields
            or completion["version"] != "mas-governed-effect-completion/1"
            or completion["effect_id"] != effect_id
            or completion["host_settlement"] != "completed"
            or completion["external_execution"] not in ("completed", "not-sent", "refused", "unknown")
            or completion["payload_disposition"] not in ("available", "withheld")
            or type(completion["output"]) is not dict):
        raise HostError("invalid governed effect completion")
    output = completion["output"]
    if (completion["payload_disposition"] == "available"
            and output.get("version") == "mas-governed-unavailable/1"):
        raise HostError("invalid governed effect completion")
    if completion["payload_disposition"] == "withheld" and output != {
            "version": "mas-governed-unavailable/1",
            "external_execution": completion["external_execution"],
            "payload_disposition": "withheld"}:
        raise HostError("invalid governed effect completion")
    return output


class Context:
    """One invocation's actor identity, original deadline and private channel."""

    def __init__(self, channel: FramedChannel, actor: str, invocation_id: str,
                 deadline_ms: int = 0):
        _key(actor, "actor")
        _key(invocation_id, "invocation_id")
        self.actor = actor
        self.invocation_id = invocation_id
        self.deadline_ms = deadline_ms
        self._channel = channel
        self._lock = threading.Lock()
        self._active = True
        self._uncertain = False

    @property
    def remaining_seconds(self) -> float | None:
        return max(0.0, self.deadline_ms / 1000 - time.time()) if self.deadline_ms else None

    def _exchange(self, command: dict[str, Any]) -> dict[str, Any]:
        with self._lock:
            if not self._active:
                raise HostError("handler context is no longer active")
            if self._uncertain:
                raise EffectUncertain("admitted effect requires reconciliation")
            self._channel.send(command)
            response = self._channel.receive()
            if response.get("type") == "response" and response.get("ok") is False:
                raise HostError(f"host rejected operation: {response.get('error', 'unknown error')}")
            return response

    def request(self, key: str, target: str, input: Any,
                output_type: Any = None) -> Any:
        _step_key(key)
        _key(target, "request target")
        response = self._exchange({"type": "request", "key": key,
                                   "target": target, "input": encode(input)})
        _success(response)
        output = response.get("output")
        validate_object(output)
        return decode(output, output_type) if output_type is not None else output

    def parallel(self, requests: Sequence[Request]) -> list[dict[str, Any]]:
        if not 1 <= len(requests) <= 8:
            raise ValueError("parallel requires 1..8 requests")
        keys = set()
        commands = []
        for request in requests:
            if not isinstance(request, Request):
                raise TypeError("parallel expects Request instances")
            _step_key(request.key)
            _key(request.target, "request target")
            if request.key in keys:
                raise ValueError("parallel requires distinct request keys")
            keys.add(request.key)
            commands.append({"key": request.key, "target": request.target,
                             "input": encode(request.input)})
        response = self._exchange({"type": "parallel", "requests": commands})
        _success(response)
        outputs = response.get("outputs")
        if type(outputs) is not list or len(outputs) != len(commands):
            raise HostError("host returned the wrong number of parallel results")
        for output in outputs:
            validate_object(output)
        return outputs

    def effect(self, key: str, descriptor: Any,
               execute: Callable[[str], EffectResult]) -> dict[str, Any]:
        _step_key(key)
        if not callable(execute):
            raise TypeError("effect requires a callback")
        command = {"type": "effect_begin", "key": key, "descriptor": encode(descriptor)}
        dumps_object(command)
        try:
            response = self._exchange(command)
        except (EOFError, OSError, TypeError, ValueError) as error:
            self._uncertain = True
            raise EffectUncertain("effect admission requires reconciliation") from error
        if response.get("type") != "effect":
            self._uncertain = True
            raise EffectUncertain("invalid effect admission requires reconciliation")
        if response.get("state") == "cached":
            output = response.get("output")
            validate_object(output)
            return output
        if (response.get("state") != "execute"
                or type(response.get("invocation_id")) is not str
                or not response["invocation_id"]):
            self._uncertain = True
            raise EffectUncertain("effect admission requires reconciliation")
        effect_id = response["invocation_id"]
        try:
            result = execute(effect_id)
            if not isinstance(result, EffectResult):
                raise TypeError("effect callback must return EffectResult")
            output, evidence = encode(result.output), encode(result.evidence)
            if not evidence:
                raise ValueError("effect evidence must be a nonempty object")
            # Validate the complete frame before attempting the durable write.
            command = {"type": "effect_complete", "key": key,
                       "output": output, "evidence": evidence}
            dumps_object(command)
            completion = self._exchange(command)
            _success(completion)
        except BaseException as error:
            self._uncertain = True
            if isinstance(error, (KeyboardInterrupt, SystemExit)):
                raise
            raise EffectUncertain("admitted effect requires reconciliation") from error
        # A successful host settlement is known even if its delivery projection
        # is malformed or withheld. Never fall back to the local raw output.
        return _completed_output(completion, effect_id, output)
