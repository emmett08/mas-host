"""Challenge notifier contracts through the exported adapter API."""
import json
import unittest
from unittest.mock import Mock, patch

from privacy_response import (CaseEvent, HttpsIncidentSink, Notification,
                              ResponseError, respond)


INVOCATION = "task-" + "1" * 64


class Connection:
    def __init__(self, response=b"HTTP/1.1 202 Accepted\r\n\r\n"):
        self.response = bytearray(response)
        self.request = b""
        self.closed = False

    def settimeout(self, timeout):
        self.timeout = timeout

    def sendall(self, request):
        self.request += request

    def recv(self, size):
        value = bytes(self.response[:size])
        del self.response[:size]
        return value

    def close(self):
        self.closed = True


class Context:
    """An exact descriptor transcript for target-binding rejection only."""
    invocation_id = INVOCATION

    def __init__(self):
        self.descriptors = {}
        self.outputs = {}

    def effect(self, key, descriptor, execute):
        raw = json.dumps(descriptor, sort_keys=True)
        if key in self.descriptors:
            if self.descriptors[key] != raw:
                raise ResponseError("effect-identity-conflict")
            return self.outputs[key]
        self.descriptors[key] = raw
        self.outputs[key] = execute("eff-" + "2" * 64).output
        return self.outputs[key]


class BoundaryTests(unittest.TestCase):
    def event(self):
        return CaseEvent("handling-withheld", "withheld", "blocked")

    def sink(self, endpoint="https://incident.example.invalid/privacy", **kwargs):
        return HttpsIncidentSink(route="data-owner", endpoint=endpoint,
                                bearer_token="synthetic-secret", **kwargs)

    def submit(self, response):
        connection = Connection(response)
        tls = Mock()
        tls.wrap_socket.return_value = connection
        with patch("privacy_response.notifier.socket.create_connection", return_value=connection), \
             patch("privacy_response.notifier.ssl.create_default_context", return_value=tls):
            result = self.sink().submit(Notification.for_invocation(INVOCATION, self.event()))
        self.assertTrue(connection.closed)
        return result

    def test_direct_notification_construction_rejects_unvalidated_header_identities(self):
        valid = Notification.for_invocation(INVOCATION, self.event())
        for field, value in (("action_id", "x\r\nAuthorization: injected"),
                             ("action_id", "privacy-notification-" + "f" * 63),
                             ("case_id", "private-person@example.invalid"),
                             ("event", {"reason": "private-value"})):
            with self.subTest(field=field, value=value), self.assertRaises(ResponseError):
                Notification(**({"case_id": valid.case_id, "action_id": valid.action_id,
                                 "event": self.event()} | {field: value}))

    def test_tls_exhausting_deadline_prevents_starting_post(self):
        connection = Connection()
        clock = [0.0]
        tls = Mock()

        def wrap(*_args, **_kwargs):
            clock[0] = 2.0
            return connection

        tls.wrap_socket.side_effect = wrap
        with patch("privacy_response.notifier.time.monotonic", side_effect=lambda: clock[0]), \
             patch("privacy_response.notifier.socket.create_connection", return_value=connection), \
             patch("privacy_response.notifier.ssl.create_default_context", return_value=tls):
            result = self.sink(timeout_seconds=1).submit(
                Notification.for_invocation(INVOCATION, self.event()))
        self.assertEqual(result.evidence["execution"], "not-sent")
        self.assertEqual(result.output["submission"], "not-sent")
        self.assertEqual(connection.request, b"")
        self.assertTrue(connection.closed)

    def test_guaranteed_refusal_survives_unusable_response_metadata(self):
        for raw, state in (
            (b"HTTP/1.1 403 Forbidden\r\ninvalid-header\r\n", "refused"),
            (b"HTTP/1.1 403 Forbidden\r\nX:", "refused"),
            (b"HTTP/1.1 429 Too Many Requests\r\nX: " + b"x" * 9000, "throttled"),
        ):
            with self.subTest(status=raw[:32]):
                result = self.submit(raw)
                self.assertEqual(result.evidence["execution"], "refused")
                self.assertEqual(result.output["submission"], state)
                self.assertEqual(result.output["recipient_delivery"], "unknown")
                self.assertEqual(result.output["response_metadata"], "withheld")
                self.assertEqual(result.output["retry_after_seconds"], 0)

    def test_changed_endpoint_cannot_reuse_an_earlier_targets_acceptance(self):
        context = Context()
        connection = Connection()
        tls = Mock()
        tls.wrap_socket.return_value = connection
        with patch("privacy_response.notifier.socket.create_connection", return_value=connection), \
             patch("privacy_response.notifier.ssl.create_default_context", return_value=tls):
            self.assertEqual(respond(context, self.event(), self.sink())["submission"], "accepted")
            with self.assertRaises(ResponseError):
                respond(context, self.event(), self.sink("https://other.example.invalid/privacy"))
        self.assertEqual(connection.request.count(b"POST "), 1)

    def test_target_rotation_after_intent_is_known_unsent_before_dispatch(self):
        sink = self.sink()
        frozen = sink.configuration_binding()

        class RotatingContext(Context):
            def effect(self, key, descriptor, execute):
                result = super().effect(key, descriptor, execute)
                if key == "privacy-case-intent":
                    sink.path = "/new-target"
                return result

        with patch("privacy_response.notifier.socket.create_connection") as connect:
            result = respond(RotatingContext(), self.event(), sink)
        self.assertNotEqual(frozen, sink.configuration_binding())
        self.assertEqual(result["submission"], "not-sent")
        self.assertEqual(result["recipient_delivery"], "unknown")
        connect.assert_not_called()

    def test_route_mismatch_is_known_unsent_and_credentials_do_not_rebind_target(self):
        sink = self.sink()
        before = sink.configuration_binding()
        sink._bearer = "rotated-synthetic-secret"
        self.assertEqual(before, sink.configuration_binding())
        event = CaseEvent("prohibited-finding", "withheld", "blocked",
                          categories=("credential",), finding_count=1)
        with patch("privacy_response.notifier.socket.create_connection") as connect:
            result = sink.submit(Notification.for_invocation(INVOCATION, event))
        self.assertEqual(result.evidence["execution"], "not-sent")
        self.assertEqual(result.output["submission"], "refused")
        connect.assert_not_called()


if __name__ == "__main__":
    unittest.main()
