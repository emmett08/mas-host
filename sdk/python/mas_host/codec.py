"""Encode the exact bounded JSON-object profile used by mas-native/1."""
from __future__ import annotations

import json
from typing import Any

MAX_FRAME_BYTES = 65_536
MAX_DEPTH = 32
MAX_INTEGER_DIGITS = 1024
# The CLI pretty-prints its wrapper and output. Indentation can expand each
# native token by up to twice the container depth, independently of frame size.
MAX_CLI_BYTES = MAX_FRAME_BYTES * (2 * MAX_DEPTH + 16)


def _string(value: str, maximum: int = MAX_FRAME_BYTES) -> int:
    if len(value) > maximum:
        raise ValueError("JSON exceeds byte limit")
    try:
        return len(value.encode("utf-8"))
    except UnicodeEncodeError as error:
        raise ValueError("JSON contains an unpaired Unicode surrogate") from error


def validate_object(value: Any) -> None:
    """Reject implicit coercion, floats, cycles and values outside MAS bounds."""
    _validate_object(value, MAX_FRAME_BYTES, MAX_DEPTH)


def _validate_object(value: Any, max_bytes: int, max_depth: int) -> None:
    if type(value) is not dict:
        raise ValueError("native values must be JSON objects")
    stack = [(value, 0, frozenset())]
    budget = max_bytes
    while stack:
        item, depth, parents = stack.pop()
        kind = type(item)
        if kind in (dict, list):
            if len(item) > max_bytes:
                raise ValueError("JSON exceeds structural limits")
            if depth >= max_depth:
                raise ValueError("JSON nesting exceeds container limit")
            if id(item) in parents:
                raise ValueError("JSON contains a cycle")
            parents = parents | {id(item)}
            budget -= 2
            if kind is dict:
                for key, child in item.items():
                    if type(key) is not str:
                        raise TypeError("JSON object keys must be strings")
                    budget -= _string(key, max_bytes) + 3
                    stack.append((child, depth + 1, parents))
            else:
                stack.extend((child, depth + 1, parents) for child in item)
                budget -= max(0, len(item) - 1)
        elif kind is str:
            budget -= _string(item, max_bytes) + 2
        elif kind is int:
            if item.bit_length() > 3402 or len(str(item).removeprefix("-")) > MAX_INTEGER_DIGITS:
                raise ValueError("JSON integers require at most 1024 decimal digits")
            budget -= len(str(item))
        elif item is None or kind is bool:
            budget -= 5 if item is False else 4
        else:
            raise TypeError("JSON supports exact integers, strings, booleans, null, objects and arrays")
        if budget < 0:
            raise ValueError("JSON exceeds 65536 bytes")


def _depth(text: str, maximum: int) -> None:
    depth = 0
    quoted = escaped = False
    for char in text:
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
        elif char == '"':
            quoted = True
        elif char in "[{":
            depth += 1
            if depth > maximum:
                raise ValueError("JSON nesting exceeds container limit")
        elif char in "]}":
            depth -= 1


def _pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key!r}")
        result[key] = value
    return result


def _integer(text: str) -> int:
    if len(text.removeprefix("-")) > MAX_INTEGER_DIGITS:
        raise ValueError("JSON integers require at most 1024 decimal digits")
    return int(text)


def _reject_number(text: str) -> Any:
    raise ValueError("JSON requires exact integer numbers; encode decimals as strings or scaled integers")


def loads_object(data: bytes | str) -> dict[str, Any]:
    """Decode without duplicate-key replacement or integer-to-float conversion."""
    return _loads_object(data, MAX_FRAME_BYTES, MAX_DEPTH)


def loads_cli_object(data: bytes | str) -> dict[str, Any]:
    """Decode the bounded pretty CLI envelope; validate its payload separately.

    The additional container belongs to the CLI wrapper, never a native frame.
    This function does not change loads_object's native limits.
    """
    return _loads_object(data, MAX_CLI_BYTES, MAX_DEPTH + 1)


def _loads_object(data: bytes | str, max_bytes: int, max_depth: int) -> dict[str, Any]:
    if isinstance(data, str):
        _string(data, max_bytes)
        data = data.encode("utf-8")
    if not isinstance(data, bytes):
        raise TypeError("JSON input must be bytes or text")
    if not 1 <= len(data) <= max_bytes:
        raise ValueError(f"JSON size must be 1..{max_bytes} bytes")
    try:
        text = data.decode("utf-8")
        _depth(text, max_depth)
        value = json.loads(text, object_pairs_hook=_pairs, parse_int=_integer,
                           parse_float=_reject_number, parse_constant=_reject_number)
    except (UnicodeDecodeError, RecursionError) as error:
        raise ValueError("invalid bounded UTF-8 JSON") from error
    _validate_object(value, max_bytes, max_depth)
    return value


def dumps_object(value: Any) -> bytes:
    """Encode an object with exact integer and Unicode semantics."""
    validate_object(value)
    data = json.dumps(value, ensure_ascii=False, allow_nan=False,
                      separators=(",", ":"), sort_keys=True).encode("utf-8")
    if len(data) > MAX_FRAME_BYTES:
        raise ValueError("JSON exceeds 65536 bytes")
    return data
