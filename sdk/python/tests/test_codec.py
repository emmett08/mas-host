"""Native JSON conformance at the limits shared by all language SDKs."""

import unittest

from mas_host.codec import dumps_object, loads_object


class CodecTests(unittest.TestCase):
    def test_exact_integer_and_unicode_round_trip(self):
        value = {
            "integer": 2**100 + 17,
            "negative": -(2**100 + 17),
            "text": "\u00e9\U0001f916",
            "other": [True, False, None, {"nested": -1}],
        }
        self.assertEqual(loads_object(dumps_object(value)), value)
        self.assertEqual(loads_object('{"text":"\\ud83e\\udd16"}'), {"text": "\U0001f916"})

    def test_integer_decimal_digit_limit(self):
        for sign in ("", "-"):
            with self.subTest(sign=sign):
                accepted = '{"n":' + sign + "9" * 1024 + "}"
                decoded = loads_object(accepted)
                self.assertEqual(loads_object(dumps_object(decoded)), decoded)
                with self.assertRaises((TypeError, ValueError)):
                    loads_object('{"n":' + sign + "9" * 1025 + "}")
                with self.assertRaises((TypeError, ValueError)):
                    dumps_object({"n": int(sign + "9" * 1025)})

    def test_frame_byte_limit_includes_object_syntax(self):
        raw = b'{"text":"' + b"a" * (65536 - len(b'{"text":""}')) + b'"}'
        self.assertEqual(len(raw), 65536)
        value = loads_object(raw)
        self.assertEqual(loads_object(dumps_object(value)), value)
        with self.assertRaises((TypeError, ValueError)):
            loads_object(raw + b" ")
        with self.assertRaises((TypeError, ValueError)):
            dumps_object({"text": value["text"] + "a"})

    def test_container_depth_counts_the_root(self):
        accepted = 0
        for _ in range(32):
            accepted = {"child": accepted}
        self.assertEqual(loads_object(dumps_object(accepted)), accepted)
        rejected = {"child": accepted}
        with self.assertRaises((TypeError, ValueError)):
            dumps_object(rejected)
        raw = '{"child":' * 33 + "0" + "}" * 33
        with self.assertRaises((TypeError, ValueError)):
            loads_object(raw)

    def test_rejects_duplicate_keys_even_when_spelled_with_escapes(self):
        for raw in ('{"x":1,"x":2}', '{"x":1,"\\u0078":2}', '{"outer":{"x":1,"x":2}}'):
            with self.subTest(raw=raw), self.assertRaises((TypeError, ValueError)):
                loads_object(raw)

    def test_rejects_non_integer_numbers_and_non_json_constants(self):
        for token in ("1.0", "1e0", "-0.0", "NaN", "Infinity", "-Infinity", "01", "+1"):
            with self.subTest(token=token), self.assertRaises((TypeError, ValueError)):
                loads_object('{"n":' + token + "}")
        for number in (1.0, float("nan"), float("inf")):
            with self.subTest(number=number), self.assertRaises((TypeError, ValueError)):
                dumps_object({"nested": [number]})

    def test_rejects_invalid_unicode_in_values_and_keys(self):
        invalid_raw = (
            b'{"value":"\xff"}',
            b'{"value":"\\ud800"}',
            b'{"value":"\\udc00"}',
            b'{"\\ud800":1}',
            b'{"value":"\\ud800\\u0041"}',
        )
        for raw in invalid_raw:
            with self.subTest(raw=raw), self.assertRaises((TypeError, ValueError, UnicodeError)):
                loads_object(raw)
        for value in ({"value": "\ud800"}, {"\udc00": 1}):
            with self.subTest(value=repr(value)), self.assertRaises((TypeError, ValueError, UnicodeError)):
                dumps_object(value)

    def test_requires_an_object_and_one_complete_document(self):
        for raw in ("", "[]", "null", "true", "1", '"text"', "{} {}", '{} trailing'):
            with self.subTest(raw=raw), self.assertRaises((TypeError, ValueError)):
                loads_object(raw)
        for value in ([], None, True, 1, "text", {1: "number key"}):
            with self.subTest(value=value), self.assertRaises((TypeError, ValueError)):
                dumps_object(value)

    def test_rejects_cycles_without_hanging(self):
        value = {}
        value["cycle"] = value
        with self.assertRaises((TypeError, ValueError)):
            dumps_object(value)


if __name__ == "__main__":
    unittest.main()
