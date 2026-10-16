"""Typed application data remains strict at the native object boundary."""

from __future__ import annotations

from dataclasses import dataclass, field, make_dataclass
from typing import Literal, Optional
import unittest

from mas_host.adaptation import adapt, decode, encode


@dataclass(frozen=True)
class Item:
    name: str
    count: int


@dataclass(frozen=True)
class Envelope:
    items: list[Item]
    by_name: dict[str, Item]
    note: Optional[str]
    mode: Literal["brief", "full"] = "brief"
    tags: list[str] = field(default_factory=list)


@dataclass(frozen=True)
class RequiredOptional:
    value: Optional[int]


@dataclass(frozen=True)
class Result:
    total: int
    actor: str


def total_handler(ctx, value: Envelope) -> Result:
    return Result(sum(item.count for item in value.items), ctx.actor)


class AdaptationTests(unittest.TestCase):
    def decode_field(self, value, annotation):
        payload_type = make_dataclass("TypedField", [("value", annotation)])
        return decode({"value": value}, payload_type).value

    def sample(self):
        return {
            "items": [{"name": "one", "count": 2}, {"name": "two", "count": 3}],
            "by_name": {"one": {"name": "one", "count": 2}},
            "note": None,
        }

    def test_nested_dataclasses_and_default_factories_round_trip(self):
        decoded = decode(self.sample(), Envelope)
        self.assertEqual(decoded.items, [Item("one", 2), Item("two", 3)])
        self.assertEqual(decoded.by_name["one"], Item("one", 2))
        self.assertIsNone(decoded.note)
        self.assertEqual(decoded.mode, "brief")
        self.assertEqual(decoded.tags, [])
        self.assertIsNot(decoded.tags, decode(self.sample(), Envelope).tags)
        self.assertEqual(decode(encode(decoded), Envelope), decoded)

    def test_missing_required_optional_field_is_still_missing(self):
        self.assertEqual(decode({"value": None}, RequiredOptional), RequiredOptional(None))
        with self.assertRaises((TypeError, ValueError)):
            decode({}, RequiredOptional)

    def test_unknown_and_missing_fields_are_rejected(self):
        for value in ({"name": "x"}, {"name": "x", "count": 1, "extra": True}):
            with self.subTest(value=value), self.assertRaises((TypeError, ValueError)):
                decode(value, Item)
        value = self.sample()
        value["items"][0]["count"] = "2"
        with self.assertRaises((TypeError, ValueError)):
            decode(value, Envelope)

    def test_primitive_types_never_coerce(self):
        for value, target in ((True, int), (1, bool), ("1", int), (1, str), (1.0, int), (1, float)):
            with self.subTest(value=value, target=target), self.assertRaises((TypeError, ValueError)):
                self.decode_field(value, target)
        self.assertIs(self.decode_field(True, bool), True)
        self.assertEqual(self.decode_field(17, int), 17)

    def test_unions_and_literals_preserve_json_types(self):
        self.assertEqual(self.decode_field("17", int | str), "17")
        self.assertEqual(self.decode_field(17, int | str), 17)
        self.assertIsNone(self.decode_field(None, int | None))
        self.assertEqual(self.decode_field("full", Literal["brief", "full"]), "full")
        for value, target in ((False, int | str), (True, Literal[1]), (1, Literal[True]), ("long", Literal["brief", "full"])):
            with self.subTest(value=value, target=target), self.assertRaises((TypeError, ValueError)):
                self.decode_field(value, target)

    def test_dictionary_keys_must_be_strings(self):
        with self.assertRaises((TypeError, ValueError)):
            decode({1: {"name": "one", "count": 1}}, dict[str, Item])
        with self.assertRaises((TypeError, ValueError)):
            encode({1: "one"})

    def test_encoding_rejects_invalid_nested_values(self):
        for value in ({"nested": [1.0]}, {"nested": [object()]}, {"text": "\ud800"}):
            with self.subTest(value=repr(value)), self.assertRaises((TypeError, ValueError, UnicodeError)):
                encode(value)

    def test_adapt_resolves_postponed_annotations_and_returns_object(self):
        class StubContext:
            actor = "counter"

        wrapped = adapt(total_handler)
        self.assertEqual(wrapped(StubContext(), self.sample()), {"total": 5, "actor": "counter"})
        invalid = self.sample()
        invalid["items"][0]["count"] = True
        with self.assertRaises((TypeError, ValueError)):
            wrapped(StubContext(), invalid)

    def test_adapt_rejects_non_object_handler_result(self):
        def handler(ctx, value: dict) -> int:
            return 3

        with self.assertRaises((TypeError, ValueError)):
            adapt(handler)(None, {})

    def test_output_validation_does_not_reconstruct_application_dataclass(self):
        constructed = []

        @dataclass
        class ObservedOutput:
            count: int

            def __post_init__(self):
                constructed.append(self.count)

        def handler(ctx, value):
            return ObservedOutput(value["count"])

        # Set actual annotation objects because this local class has no module
        # name from which postponed annotations could be resolved.
        handler.__annotations__ = {"value": dict, "return": ObservedOutput}
        wrapped = adapt(handler)
        self.assertEqual(wrapped(None, {"count": 3}), {"count": 3})
        self.assertEqual(constructed, [3])
        with self.assertRaises((TypeError, ValueError)):
            wrapped(None, {"count": "three"})
        self.assertEqual(constructed, [3, "three"])


if __name__ == "__main__":
    unittest.main()
