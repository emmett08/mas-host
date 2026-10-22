"""Exercise safe response contracts, endpoint failures and retained-action replay."""
import copy
import json
import socket
import unittest
from unittest.mock import patch

from mas_host import EffectResult
from privacy_response import (CaseEvent, Notification, ResponseError, respond,
                              HttpsIncidentSink, MemorySink, SubmissionUncertain)


INVOCATION = "task-" + "1" * 64


class Socket:
    def __init__(self, response=b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n"):
        self.response = bytearray(response)
        self.request = b""
        self.closed = False

    def settimeout(self, _timeout):
        pass

    def sendall(self, request):
        self.request += request

    def recv(self, size):
        part = bytes(self.response[:size])
        del self.response[:size]
        return part

    def close(self):
        self.closed = True


class RetainedContext:
    """A deterministic effect transcript, not a substitute for the host test."""
    invocation_id = INVOCATION

    def __init__(self, retained=None, revoked=False):
        self.retained = retained if retained is not None else {}
        self.descriptors = {}
        self.revoked = revoked

    def effect(self, key, descriptor, execute):
        if self.revoked:
            raise ResponseError("current-authority-denied")
        raw = json.dumps(descriptor, sort_keys=True)
        if key in self.descriptors and self.descriptors[key] != raw:
            raise ResponseError("effect-identity-conflict")
        self.descriptors[key] = raw
        if key in self.retained:
            value = self.retained[key]
            if value is None:
                raise SubmissionUncertain("original-effect-unresolved")
            return copy.deepcopy(value)
        self.retained[key] = None
        result = execute("eff-" + "2" * 64)
        self.retained[key] = copy.deepcopy(result.output)
        return copy.deepcopy(result.output)


class ContractTests(unittest.TestCase):
    def test_default_withheld_event_does_not_infer_pii_or_exposure(self):
        event = CaseEvent("handling-withheld", "withheld", "blocked")
        notification = Notification.for_invocation(INVOCATION, event)
        payload = notification.payload()
        self.assertEqual(payload["categories"], [])
        self.assertEqual(payload["finding_count"], 0)
        self.assertEqual(payload["route"], "data-owner")
        self.assertEqual(payload["reason"], "handling-withheld")
        self.assertLessEqual(len(notification.encoded()), 2048)
        self.assertEqual(set(payload), {"version", "template", "case_id", "action_id", "event_version",
                                        "reason", "disposition", "exposure", "categories", "finding_count", "route"})

    def test_invalid_private_metadata_counts_and_routes_reject(self):
        for kwargs in ({"reason": "person@example.invalid"}, {"disposition": "permit"},
                       {"exposure": "person@example.invalid"}, {"categories": ("private-value",)},
                       {"finding_count": True}, {"finding_count": 129}, {"event_version": 0},
                       {"categories": ("email",), "finding_count": 0},
                       {"reason": "prohibited-finding"}, {"reason": ["private-value"]},
                       {"disposition": ["private-value"]}, {"exposure": ["private-value"]},
                       {"reason": "suspected-exposure", "exposure": "blocked"}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ResponseError):
                values = {"reason": "handling-withheld", "disposition": "withheld", "exposure": "blocked"}
                CaseEvent(**(values | kwargs))

    def test_logical_action_stable_and_host_scope_event_route_separate(self):
        event = CaseEvent("handling-withheld", "withheld", "blocked")
        same = Notification.for_invocation(INVOCATION, event)
        self.assertEqual(same, Notification.for_invocation(INVOCATION, event))
        other = Notification.for_invocation("task-" + "2" * 64, event)
        self.assertNotEqual(same.case_id, other.case_id)
        self.assertNotEqual(same.action_id, other.action_id)
        version = Notification.for_invocation(INVOCATION, CaseEvent("handling-withheld", "withheld", "blocked", 2))
        self.assertEqual(same.case_id, version.case_id)
        self.assertNotEqual(same.action_id, version.action_id)
        route = Notification.for_invocation(INVOCATION, CaseEvent("coverage-failure", "hold", "blocked"))
        self.assertNotEqual(same.action_id, route.action_id)
        with self.assertRaises(ResponseError):
            Notification.for_invocation("untrusted-request-id", event)

    def test_categories_choose_symbolic_route_without_supplied_recipient(self):
        self.assertEqual(CaseEvent("prohibited-finding", "withheld", "blocked", categories=("credential",), finding_count=1).route,
                         "security-response")
        self.assertEqual(CaseEvent("coverage-failure", "hold", "blocked").route, "service-operations")

    def test_direct_notification_constructor_rejects_header_injection_and_identity_substitution(self):
        event = CaseEvent("handling-withheld", "withheld", "blocked")
        notification = Notification.for_invocation(INVOCATION, event)
        for changes in ({"case_id": "private-value"}, {"action_id": "x\r\nAuthorization: injected"},
                        {"action_id": "privacy-notification-" + "0" * 64}, {"event": {}},
                        {"case_id": ["private-value"]}, {"action_id": None}):
            with self.subTest(changes=changes), patch("privacy_response.notifier.socket.create_connection") as connect:
                with self.assertRaisesRegex(ResponseError, "^privacy-response-contract-invalid$"):
                    Notification(**({"case_id": notification.case_id, "action_id": notification.action_id,
                                     "event": event} | changes))
                connect.assert_not_called()


class FlowTests(unittest.TestCase):
    def test_intent_precedes_send_and_same_action_replays_after_restart(self):
        event = CaseEvent("handling-withheld", "withheld", "blocked")
        context = RetainedContext()
        sink = MemorySink()
        first = respond(context, event, sink)
        self.assertEqual(list(context.retained), ["privacy-case-intent", "privacy-notification"])
        self.assertEqual(first["submission"], "accepted")
        self.assertEqual(len(sink.actions), 1)
        restarted = RetainedContext(context.retained)
        self.assertEqual(respond(restarted, event, sink), first)
        self.assertEqual(len(sink.actions), 1)

    def test_missing_or_revoked_case_intent_cannot_start_notification(self):
        event = CaseEvent("handling-withheld", "withheld", "blocked")
        for context in (RetainedContext(revoked=True), RetainedContext({"privacy-case-intent": {"version": "mas-governed-unavailable/1"}})):
            sink = MemorySink()
            with self.assertRaises(ResponseError):
                respond(context, event, sink)
            self.assertEqual(sink.actions, {})

    def test_lost_acceptance_retains_original_uncertainty_without_resend(self):
        class Lost:
            calls = 0
            def configuration_binding(self):
                return "0" * 64
            def submit(self, _notification):
                self.calls += 1
                raise SubmissionUncertain("lost-acknowledgement")
        event = CaseEvent("handling-withheld", "withheld", "blocked")
        context, sink = RetainedContext(), Lost()
        for restarted in (context, RetainedContext(context.retained)):
            with self.assertRaises(SubmissionUncertain):
                respond(restarted, event, sink)
        self.assertEqual(sink.calls, 1)
        self.assertEqual(context.retained["privacy-case-intent"]["case_retained"], True)

    def test_flood_capacity_is_explicit_refusal_and_dedup_does_not_spend(self):
        sink = MemorySink(max_actions=1)
        event = CaseEvent("handling-withheld", "withheld", "blocked")
        notification = Notification.for_invocation(INVOCATION, event)
        self.assertEqual(sink.submit(notification).output["submission"], "accepted")
        self.assertEqual(sink.submit(notification).output["submission"], "accepted")
        other = Notification.for_invocation("task-" + "2" * 64, event)
        self.assertEqual(sink.submit(other).output["submission"], "refused")
        self.assertEqual(len(sink.actions), 1)


class HttpsTests(unittest.TestCase):
    def sink(self):
        return HttpsIncidentSink(route="data-owner", endpoint="https://incident.example.invalid/privacy",
                                bearer_token="synthetic-secret")

    def submit(self, connection):
        context = unittest.mock.Mock()
        context.wrap_socket.return_value = connection
        with patch("privacy_response.notifier.socket.create_connection", return_value=connection), \
             patch("privacy_response.notifier.ssl.create_default_context", return_value=context):
            result = self.sink().submit(Notification.for_invocation(INVOCATION, CaseEvent("handling-withheld", "withheld", "blocked")))
        self.assertTrue(connection.closed)
        return result

    def test_https_config_rejects_dynamic_credential_injection_and_unapproved_contract(self):
        for changes in ({"endpoint": "http://incident.example.invalid"}, {"endpoint": "https://x@y/path"},
                        {"endpoint": "https://incident.example.invalid/?target=private"},
                        {"endpoint": "https://incident.example.invalid:444/"}, {"endpoint": 3},
                        {"bearer_token": "x\r\nX: y"}, {"timeout_seconds": 31},
                        {"route": "person@example.invalid"}, {"delivery_contract": "any"}):
            with self.subTest(changes=changes), self.assertRaises(ResponseError):
                HttpsIncidentSink(**({"route": "data-owner", "endpoint": "https://incident.example.invalid/", "bearer_token": "synthetic"} | changes))

    def test_endpoint_acceptance_is_separate_from_recipient_delivery_and_body(self):
        connection = Socket(b"HTTP/1.1 202 Accepted\r\nContent-Length: 999999999\r\n\r\nprivate-person@example.invalid")
        result = self.submit(connection)
        self.assertEqual(result.evidence["execution"], "completed")
        self.assertEqual(result.output["submission"], "accepted")
        self.assertEqual(result.output["recipient_delivery"], "unknown")
        self.assertNotIn("private-person", repr(result))
        self.assertTrue(connection.response.startswith(b"private-person"))
        self.assertIn(b"Idempotency-Key: privacy-notification-", connection.request)
        self.assertNotIn(b"person@example.invalid", connection.request)

    def test_known_acceptance_survives_invalid_or_oversized_metadata(self):
        for raw in (b"HTTP/1.1 201 Created\r\ninvalid-header\r\n", b"HTTP/1.1 200 OK\r\nX: " + b"x" * 9000,
                    b"HTTP/1.1 200 OK\r\nX: 1\r\nX: 2\r\n\r\n"):
            with self.subTest(raw=raw[:32]):
                result = self.submit(Socket(raw))
                self.assertEqual(result.output["submission"], "accepted")
                self.assertEqual(result.evidence["execution"], "completed")
                self.assertEqual(result.output["response_metadata"], "withheld")

    def test_throttling_and_known_refusal_are_retained_without_automatic_retry(self):
        for raw, state, after in ((b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 9999\r\n\r\n", "throttled", 3600),
                                  (b"HTTP/1.1 403 Forbidden\r\n\r\n", "refused", 0)):
            connection = Socket(raw)
            result = self.submit(connection)
            self.assertEqual(result.output["submission"], state)
            self.assertEqual(result.output["retry_after_seconds"], after)
            self.assertEqual(result.evidence["execution"], "refused")
            self.assertEqual(connection.request.count(b"POST "), 1)

    def test_redirect_server_failure_and_lost_response_remain_uncertain(self):
        for raw in (b"HTTP/1.1 302 Found\r\nLocation: https://other.invalid\r\n\r\n",
                    b"HTTP/1.1 500 Failed\r\n\r\n", b"HTTP/1.1 408 Timeout\r\n\r\n", b"", b"invalid"):
            connection = Socket(raw)
            with self.subTest(raw=raw), self.assertRaises(SubmissionUncertain):
                self.submit(connection)
            self.assertTrue(connection.closed)
            self.assertEqual(connection.request.count(b"POST "), 1)

    def test_connection_failure_is_proved_unsent_without_credential_or_endpoint_diagnostics(self):
        with patch("privacy_response.notifier.socket.create_connection", side_effect=OSError("private-host-secret")):
            result = self.sink().submit(Notification.for_invocation(INVOCATION, CaseEvent("handling-withheld", "withheld", "blocked")))
        self.assertEqual(result.output["submission"], "not-sent")
        self.assertEqual(result.evidence["execution"], "not-sent")
        self.assertNotIn("private-host-secret", repr(result))

    def test_adapter_concurrency_rejection_is_unsent(self):
        sink = self.sink()
        sink._active.acquire()
        try:
            result = sink.submit(Notification.for_invocation(INVOCATION, CaseEvent("handling-withheld", "withheld", "blocked")))
        finally:
            sink._active.release()
        self.assertEqual(result.evidence["execution"], "not-sent")

    def test_expired_connect_or_tls_deadline_cannot_start_a_post(self):
        for times in ([0, 6], [0, 0, 6]):
            connection = Socket()
            context = unittest.mock.Mock()
            context.wrap_socket.return_value = connection
            with self.subTest(times=times), \
                 patch("privacy_response.notifier.socket.create_connection", return_value=connection), \
                 patch("privacy_response.notifier.ssl.create_default_context", return_value=context), \
                 patch("privacy_response.notifier.time.monotonic", side_effect=times):
                result = self.sink().submit(Notification.for_invocation(INVOCATION, CaseEvent("handling-withheld", "withheld", "blocked")))
            self.assertEqual(result.evidence["execution"], "not-sent")
            self.assertEqual(connection.request, b"")
            self.assertTrue(connection.closed)


if __name__ == "__main__":
    unittest.main()
