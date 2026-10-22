"""Challenge durable retry identities, time bounds, revocation and uncertainty."""
import copy
import unittest
from unittest.mock import patch

from mas_host import EffectResult
from privacy_response import (CaseEvent, HttpsReconciledIncidentSink, Notification,
                              ResponseError, RetryPolicy, SubmissionUncertain, respond_queued,
                              AcknowledgementPolicy, respond_until_acknowledged)
from test_response import RetainedContext, Socket, INVOCATION

EVENT = CaseEvent("handling-withheld", "withheld", "blocked")


class Clock:
    now = 1_000
    def time(self):
        return self.now
    def sleep(self, seconds):
        self.now += seconds


class Sink:
    def __init__(self, states, lookup="unknown"):
        self.states = iter(states)
        self.calls = []
        self.lookups = 0
        self.lookup = lookup
    def configuration_binding(self):
        return "0" * 64
    def submit(self, notification):
        self.calls.append(notification)
        state = next(self.states)
        if state == "uncertain":
            raise SubmissionUncertain("private-supplied-diagnostic")
        return EffectResult({"submission": state, "recipient_delivery": "unknown",
                             "retry_after_seconds": 2 if state == "throttled" else 0,
                             "response_metadata": "withheld"},
                            {"execution": "completed" if state == "accepted" else "not-sent",
                             "execution_scope": "notification-endpoint-submission"})
    def reconcile(self, _notification):
        self.lookups += 1
        return self.lookup


class LifecycleTests(unittest.TestCase):
    def run_queue(self, sink, ctx=None, clock=None, policy=RetryPolicy()):
        ctx = ctx or RetainedContext()
        clock = clock or Clock()
        result = respond_queued(ctx, EVENT, sink, policy, clock=clock.time, sleep=clock.sleep)
        return result, ctx, clock

    def test_throttle_retries_same_action_and_restart_replays_without_new_send(self):
        sink = Sink(["throttled", "accepted"])
        result, ctx, clock = self.run_queue(sink)
        self.assertEqual(result["submission"], "accepted")
        self.assertEqual(len(sink.calls), 2)
        self.assertEqual(sink.calls[0], sink.calls[1])
        self.assertGreaterEqual(clock.now, 1_002)
        restarted = RetainedContext(copy.deepcopy(ctx.retained))
        replay, _, _ = self.run_queue(sink, restarted, clock)
        self.assertEqual(replay, result)
        self.assertEqual(len(sink.calls), 2)

    def test_unknown_lookup_does_not_resubmit_and_acceptance_reconciles_same_action(self):
        for lookup, expected in (("unknown", "uncertain"), ("accepted", "accepted")):
            with self.subTest(lookup=lookup):
                sink = Sink(["uncertain"], lookup)
                result, ctx, _ = self.run_queue(sink)
                self.assertEqual(result["submission"], expected)
                self.assertEqual(len(sink.calls), 1)
                self.assertEqual(sink.lookups, 1)
                self.assertEqual(ctx.retained["privacy-queue-submit-0"]["submission"], "uncertain")
                self.assertNotIn("private-supplied", repr(ctx.retained))

    def test_only_authoritative_absence_allows_retry_after_ambiguous_send(self):
        sink = Sink(["uncertain", "accepted"], "not-accepted")
        result, _, _ = self.run_queue(sink)
        self.assertEqual(result["submission"], "accepted")
        self.assertEqual(len(sink.calls), 2)
        self.assertEqual(sink.calls[0].action_id, sink.calls[1].action_id)

    def test_expiry_and_attempt_quota_stop_without_further_submission(self):
        sink = Sink(["throttled"])
        result, _, _ = self.run_queue(sink, policy=RetryPolicy(max_age_seconds=1))
        self.assertEqual(result["submission"], "expired")
        self.assertEqual(len(sink.calls), 1)
        sink = Sink(["not-sent", "not-sent"])
        result, _, _ = self.run_queue(sink, policy=RetryPolicy(max_attempts=2))
        self.assertEqual(result["submission"], "exhausted")
        self.assertEqual(len(sink.calls), 2)

    def test_revocation_after_queue_intent_prevents_fresh_timer_or_submission(self):
        class Revoking(RetainedContext):
            def effect(self, key, descriptor, execute):
                result = super().effect(key, descriptor, execute)
                if key == "privacy-queue-intent":
                    self.revoked = True
                return result
        sink = Sink(["accepted"])
        with self.assertRaises(ResponseError):
            self.run_queue(sink, Revoking())
        self.assertEqual(sink.calls, [])

    def test_unavailable_or_malformed_retained_state_cannot_start_or_retry_send(self):
        for state in ({"version": "mas-governed-unavailable/1"},
                      {"queued_at_ms": True, "expires_at_ms": 600001}):
            sink = Sink(["accepted"])
            with self.subTest(state=state), self.assertRaises(ResponseError):
                self.run_queue(sink, RetainedContext({"privacy-queue-intent": state}))
            self.assertEqual(sink.calls, [])
        sink = Sink(["uncertain"], "invalid")
        with self.assertRaises(ResponseError):
            self.run_queue(sink)
        self.assertEqual(len(sink.calls), 1)

    def test_policy_validates_boolean_limits_and_fresh_policy_conflicts(self):
        for values in ({"max_attempts": True}, {"max_attempts": 9}, {"backoff_seconds": 0},
                       {"max_age_seconds": 3601}):
            with self.subTest(values=values), self.assertRaises(ResponseError):
                RetryPolicy(**values)
        ctx = RetainedContext()
        self.run_queue(Sink(["accepted"]), ctx)
        with self.assertRaises(ResponseError):
            self.run_queue(Sink(["accepted"]), ctx, policy=RetryPolicy(max_attempts=2))


class LookupTests(unittest.TestCase):
    def sink(self):
        return HttpsReconciledIncidentSink(route="data-owner", endpoint="https://incident.example.invalid/privacy",
                  bearer_token="synthetic", lookup_path="/by-action/",
                  delivery_contract="idempotency-key-authoritative-lookup/1")

    def test_exact_action_same_origin_lookup_preserves_unknown_statuses(self):
        notification = Notification.for_invocation(INVOCATION, EVENT)
        for status, expected in ((200, "accepted"), (204, "accepted"), (404, "not-accepted"),
                                 (500, "unknown"), (302, "unknown"), (403, "unknown")):
            connection = Socket(f"HTTP/1.1 {status} fixture\r\n".encode())
            tls = unittest.mock.Mock()
            tls.wrap_socket.return_value = connection
            with self.subTest(status=status), patch("privacy_response.reconciliation.socket.create_connection", return_value=connection), \
                 patch("privacy_response.reconciliation.ssl.create_default_context", return_value=tls):
                result = self.sink().reconcile(notification)
            self.assertEqual(result, expected)
            self.assertIn(("GET /by-action/" + notification.action_id).encode(), connection.request)
            self.assertNotIn(b"POST", connection.request)
            self.assertTrue(connection.closed)

    def test_unqualified_lookup_contract_or_dynamic_path_rejects(self):
        for values in ({"delivery_contract": "any"}, {"lookup_path": "https://other.invalid/"},
                       {"lookup_path": "/x?secret/"}, {"lookup_path": "/../"}):
            with self.subTest(values=values), self.assertRaises(ResponseError):
                HttpsReconciledIncidentSink(**({"route": "data-owner", "endpoint": "https://incident.example.invalid/privacy",
                  "bearer_token": "synthetic", "lookup_path": "/by-action/",
                  "delivery_contract": "idempotency-key-authoritative-lookup/1"} | values))


if __name__ == "__main__":
    unittest.main()


class AcknowledgementTests(unittest.TestCase):
    def test_acknowledgement_and_escalation_keep_distinct_logical_actions(self):
        for acknowledgement, escalated in (("acknowledged", False), ("pending", True), ("unknown", True)):
            sink = Sink(["accepted", "accepted"])
            sink.acknowledged = lambda _notification: acknowledgement
            ctx, clock = RetainedContext(), Clock()
            result = respond_until_acknowledged(ctx, EVENT, sink, RetryPolicy(),
                       AcknowledgementPolicy(wait_seconds=2, poll_seconds=1), clock=clock.time, sleep=clock.sleep)
            self.assertEqual(len(sink.calls), 2 if escalated else 1)
            if escalated:
                self.assertEqual(sink.calls[0].case_id, sink.calls[1].case_id)
                self.assertNotEqual(sink.calls[0].action_id, sink.calls[1].action_id)
                self.assertEqual(sink.calls[1].payload()["template"], "privacy-handling-escalation/1")
                self.assertGreaterEqual(clock.now, 1002)
            replay = respond_until_acknowledged(RetainedContext(copy.deepcopy(ctx.retained)), EVENT, sink,
                       RetryPolicy(), AcknowledgementPolicy(wait_seconds=2, poll_seconds=1), clock=clock.time, sleep=clock.sleep)
            self.assertEqual(result, replay)
            self.assertEqual(len(sink.calls), 2 if escalated else 1)

    def test_revocation_between_acknowledgement_and_escalation_stops_new_admission(self):
        ctx, clock, sink = RetainedContext(), Clock(), Sink(["accepted"])
        def acknowledgement(_notification):
            ctx.revoked = True
            return "pending"
        sink.acknowledged = acknowledgement
        with self.assertRaises(ResponseError):
            respond_until_acknowledged(ctx, EVENT, sink, RetryPolicy(),
                AcknowledgementPolicy(wait_seconds=1, poll_seconds=1), clock=clock.time, sleep=clock.sleep)
        self.assertEqual(len(sink.calls), 1)


class LateRecoveryTests(unittest.TestCase):
    def test_known_accepted_attempt_survives_recovery_after_queue_expiry(self):
        sink, ctx, clock = Sink(["accepted"]), RetainedContext(), Clock()
        first = respond_queued(ctx, EVENT, sink, clock=clock.time, sleep=clock.sleep)
        clock.now += 1000
        again = respond_queued(RetainedContext(copy.deepcopy(ctx.retained)), EVENT, sink,
                               clock=clock.time, sleep=clock.sleep)
        self.assertEqual(first, again)
        self.assertEqual(len(sink.calls), 1)
