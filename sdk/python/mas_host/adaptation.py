"""Adapt explicitly annotated dataclasses and JSON types without coercion."""
from __future__ import annotations

import dataclasses
import inspect
import types
from collections.abc import Callable
from typing import Any, Literal, Union, get_args, get_origin, get_type_hints

from .codec import MAX_DEPTH, validate_object


def _encode(value: Any, depth: int, parents: frozenset[int]) -> Any:
    if depth > MAX_DEPTH:
        raise ValueError("native value nesting exceeds 32 containers")
    if dataclasses.is_dataclass(value) and not isinstance(value, type):
        if id(value) in parents:
            raise ValueError("native value contains a cycle")
        parents = parents | {id(value)}
        result = {field.name: _encode(getattr(value, field.name), depth + 1, parents)
                  for field in dataclasses.fields(value)}
        _decode(result, type(value), "payload", construct=False)
        return result
    if type(value) is dict:
        if id(value) in parents:
            raise ValueError("native value contains a cycle")
        parents = parents | {id(value)}
        return {key: _encode(child, depth + 1, parents) for key, child in value.items()}
    if type(value) is list:
        if id(value) in parents:
            raise ValueError("native value contains a cycle")
        parents = parents | {id(value)}
        return [_encode(child, depth + 1, parents) for child in value]
    if value is None or type(value) in (str, int, bool):
        return value
    raise TypeError("native values require dataclasses or exact JSON types; encode decimals explicitly")


def encode(value: Any) -> dict[str, Any]:
    """Convert dataclass instances recursively; every public payload is an object."""
    result = _encode(value, 0, frozenset())
    validate_object(result)
    return result


def _decode(value: Any, expected: Any, location: str, *, construct: bool = True) -> Any:
    origin = get_origin(expected)
    arguments = get_args(expected)
    if expected in (Any, object):
        return value
    if origin in (Union, types.UnionType):
        matches = []
        for choice in arguments:
            try:
                _decode(value, choice, location, construct=False)
                matches.append(choice)
            except (TypeError, ValueError):
                pass
        if len(matches) != 1:
            raise TypeError(f"{location}: value must match exactly one union variant")
        return _decode(value, matches[0], location, construct=construct)
    if origin is Literal:
        if not any(type(value) is type(choice) and value == choice for choice in arguments):
            raise TypeError(f"{location}: value is outside Literal choices")
        return value
    if expected is None or expected is type(None):
        if value is not None:
            raise TypeError(f"{location}: expected null")
        return None
    if dataclasses.is_dataclass(expected) and isinstance(expected, type):
        if type(value) is not dict:
            raise TypeError(f"{location}: expected dataclass object")
        fields = {field.name: field for field in dataclasses.fields(expected)}
        if any(not field.init for field in fields.values()):
            raise TypeError("typed dataclasses require init=True on all fields")
        unknown = value.keys() - fields.keys()
        if unknown:
            raise ValueError(f"{location}: unknown dataclass fields: {sorted(unknown)}")
        hints = get_type_hints(expected)
        supplied = {}
        for name, field in fields.items():
            if name in value:
                supplied[name] = _decode(value[name], hints[name], f"{location}.{name}",
                                        construct=construct)
            elif field.default is dataclasses.MISSING and field.default_factory is dataclasses.MISSING:
                raise ValueError(f"{location}: missing dataclass field {name}")
        if not construct:
            return value
        result = expected(**supplied)
        # Defaults and __post_init__ remain application code; require the
        # resulting instance to obey the same serialisation profile.
        encode(result)
        return result
    if origin is list or expected is list:
        if type(value) is not list:
            raise TypeError(f"{location}: expected array")
        element = arguments[0] if arguments else Any
        return [_decode(child, element, f"{location}[{index}]", construct=construct)
                for index, child in enumerate(value)]
    if origin is dict or expected is dict:
        if type(value) is not dict:
            raise TypeError(f"{location}: expected object")
        key_type, element = arguments if arguments else (str, Any)
        if key_type is not str:
            raise TypeError("typed dictionary keys must be str")
        return {key: _decode(child, element, f"{location}.{key}", construct=construct)
                for key, child in value.items()}
    if expected in (str, int, bool):
        if type(value) is not expected:
            raise TypeError(f"{location}: expected {expected.__name__}")
        return value
    raise TypeError(f"unsupported payload annotation: {expected!r}")


def decode(value: dict[str, Any], expected: Any = dict) -> Any:
    """Construct typed payloads with strict primitives and dataclass fields."""
    validate_object(value)
    return _decode(value, expected, "payload")


def adapt(handler: Callable[..., Any]) -> Callable[[Any, dict[str, Any]], dict[str, Any]]:
    """Bind a synchronous two-argument handler; unannotated payloads use dicts."""
    signature = inspect.signature(handler)
    parameters = list(signature.parameters.values())
    if len(parameters) != 2 or any(parameter.kind not in (
            inspect.Parameter.POSITIONAL_ONLY, inspect.Parameter.POSITIONAL_OR_KEYWORD)
            for parameter in parameters):
        raise TypeError("handler signature must be (context, input)")
    if inspect.iscoroutinefunction(handler):
        raise TypeError("async handlers are not supported by the synchronous worker contract")
    hints = get_type_hints(handler)
    input_type = hints.get(parameters[1].name, dict)
    output_type = hints.get("return")

    def invoke(context: Any, payload: dict[str, Any]) -> dict[str, Any]:
        result = handler(context, decode(payload, input_type))
        encoded = encode(result)
        if output_type is not None:
            _decode(encoded, output_type, "output", construct=False)
        return encoded

    return invoke
