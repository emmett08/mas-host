"""Provider error, timing and disclosure controls using actual loopback HTTP."""

import copy
from email.utils import formatdate
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import socket
import sys
import threading
import time
import traceback
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from provider import observe, schedule


NOW = 1800000000000


class ContractTests(unittest.TestCase):
    def observation(self, provider="github", operation="read", status=429, headers=(), **kwargs):
        return observe(provider, "registered/account-42/core", operation, now_ms=NOW,
                       http_status=status, headers=headers, **kwargs)

    def decision(self, value, **kwargs):
        return schedule(value, now_ms=NOW, deadline_ms=NOW + 7200000, **kwargs)

    def test_primary_reset_and_retry_after_are_both_lower_bounds(self):
        value = self.observation(headers=[("X-RateLimit-Remaining", "0"),
            ("X-RateLimit-Reset", str(NOW // 1000 + 200)), ("Retry-After", "300")])
        self.assertEqual(value["throttle_kind"], "primary")
        self.assertEqual(value["retry_not_before_ms"], NOW + 300000)
        self.assertEqual(value["retry_after_not_before_ms"], NOW + 300000)
        self.assertEqual(value["rate_limit_reset_ms"], NOW + 200000)
        decision = self.decision(value, jitter_ms=123)
        self.assertEqual(decision["action"], "defer")
        self.assertGreaterEqual(decision["not_before_ms"], NOW + 300000)
        self.assertFalse(decision["execution_permission"])

    def test_secondary_limit_has_one_minute_floor_and_bounded_backoff(self):
        value = self.observation(status=403, headers=[("Retry-After", "1")])
        self.assertEqual(value["throttle_kind"], "secondary-or-unspecified")
        self.assertEqual(self.decision(value)["not_before_ms"], NOW + 60000)
        self.assertEqual(self.decision(value, attempt=2)["not_before_ms"], NOW + 240000)
        self.assertEqual(self.decision(value, attempt=8)["action"], "hold")

    def test_duplicate_observation_does_not_renew_its_cooldown(self):
        value = self.observation(status=403, headers=[("Retry-After", "1")])
        first = self.decision(value, jitter_ms=100)
        repeat = schedule(value, now_ms=NOW + 30000, deadline_ms=NOW + 7200000, jitter_ms=100)
        self.assertEqual(first["not_before_ms"], repeat["not_before_ms"])
        eligible = schedule(value, now_ms=NOW + 120000, deadline_ms=NOW + 7200000, jitter_ms=100)
        self.assertEqual(eligible["not_before_ms"], NOW + 120000)

    def test_primary_without_reset_and_duplicate_or_invalid_hints_hold(self):
        for headers in ([('X-RateLimit-Remaining', '0')],
                        [('Retry-After', '1'), ('retry-after', '2')],
                        [('Retry-After', 'person@example.invalid')],
                        [('Retry-After', '-1')], [('Retry-After', '1.5')],
                        [('Retry-After', '999999999999999999999999')]):
            with self.subTest(headers=headers):
                value = self.observation(headers=headers)
                self.assertEqual(value["retry_hint_state"], "invalid")
                self.assertEqual(self.decision(value)["action"], "hold")

    def test_http_date_is_parsed_without_preserving_original_header(self):
        header = formatdate(NOW // 1000 + 90, usegmt=True)
        value = self.observation("kubernetes", headers=[("Retry-After", header)])
        self.assertEqual(value["retry_not_before_ms"], NOW + 90000)
        self.assertNotIn(header, json.dumps(value))

    def test_large_hint_is_never_clamped_to_an_earlier_retry(self):
        value = self.observation(headers=[("Retry-After", "7200")])
        decision = self.decision(value)
        self.assertEqual(decision["action"], "expired")
        self.assertIsNone(decision["not_before_ms"])
        decision = schedule(value, now_ms=NOW, deadline_ms=NOW + 10800000)
        self.assertEqual(decision["action"], "hold")
        self.assertIsNone(decision["not_before_ms"])

    def test_unknown_mutations_reconcile_even_after_original_deadline(self):
        cases = [self.observation(operation="mutation", status=500),
                 self.observation(operation="mutation", status=408),
                 self.observation(operation="mutation", status=403),
                 observe("github", "registered/account-42", "mutation", now_ms=NOW,
                         transport="outcome-unknown")]
        for value in cases:
            with self.subTest(value=value):
                result = schedule(value, now_ms=NOW, deadline_ms=NOW)
                self.assertEqual(result["action"], "reconcile")
                self.assertTrue(result["reconciliation_required"])
                self.assertFalse(result["execution_permission"])
                self.assertIsNone(result["not_before_ms"])

    def test_known_throttle_refusal_and_established_not_sent_can_defer(self):
        value = self.observation("kubernetes", "mutation", headers=[("Retry-After", "2")])
        self.assertEqual(value["mutation_disposition"], "known-throttle-refusal")
        self.assertEqual(self.decision(value)["action"], "defer")
        unsent = observe("github", "registered/account-42", "mutation", now_ms=NOW,
                         transport="not-dispatched")
        self.assertEqual(self.decision(unsent)["action"], "defer")
        self.assertTrue(self.decision(unsent)["requires_fresh_admission"])

    def test_mutation_acceptance_preserves_reconciliation_obligation(self):
        value = self.observation(operation="mutation", status=202)
        decision = self.decision(value)
        self.assertEqual(decision["action"], "finish")
        self.assertTrue(decision["reconciliation_required"])
        expired = schedule(value, now_ms=NOW, deadline_ms=NOW)
        self.assertTrue(expired["reconciliation_required"])

    def test_gone_watch_records_lost_interval_and_requires_new_snapshot(self):
        value = self.observation("kubernetes", "watch", status=410, watch_cursor="private-resource-version")
        result = self.decision(value)
        self.assertEqual(result["action"], "relist")
        self.assertTrue(result["coverage_gap"])
        self.assertFalse(result["new_snapshot_repairs_history"])
        self.assertTrue(result["requires_fresh_admission"])
        self.assertNotIn("private-resource-version", json.dumps(value))
        ordinary = self.observation("kubernetes", "read", status=410)
        self.assertEqual(self.decision(ordinary)["action"], "refused")

    def test_conditional_response_and_redirect_have_distinct_actions(self):
        self.assertEqual(self.decision(self.observation(status=304))["action"], "finish")
        redirect = self.observation(status=302, headers=[("Location", "https://secret.invalid/token")])
        self.assertEqual(self.decision(redirect)["action"], "refused")
        self.assertNotIn("secret", json.dumps(redirect))

    def test_body_and_unapproved_headers_never_enter_the_result(self):
        value = self.observation(headers=[("Retry-After", "3"),
            ("Authorization", "Bearer fixture-private-token"),
            ("X-Request-Body", "person@example.invalid")])
        text = json.dumps(value) + json.dumps(self.decision(value))
        self.assertNotIn("fixture-private-token", text)
        self.assertNotIn("person@example.invalid", text)
        self.assertFalse(value["body_retained"])

    def test_unbounded_or_malformed_contract_values_are_rejected(self):
        invalid = [dict(headers=[("X", "x" * 4097)]), dict(headers=[("X", "a")] * 65),
                   dict(http_status=True), dict(quota_scope="Bearer private token"),
                   dict(headers=[("Retry-After", "1\r\nAuthorization: private")])]
        for update in invalid[:-1]:
            args = dict(provider="github", quota_scope="registered/42", operation="read",
                        now_ms=NOW, http_status=429)
            args.update(update)
            with self.subTest(update=update), self.assertRaises(ValueError):
                observe(**args)
        value = self.observation(headers=invalid[-1]["headers"])
        self.assertEqual(self.decision(value)["action"], "hold")

    def test_exception_chains_do_not_preserve_private_header_text(self):
        private = "fixture-" + "private-token\ud800"
        try:
            self.observation(headers=[("Retry-After", private)])
        except ValueError:
            text = traceback.format_exc()
        self.assertNotIn("fixture-private-token", text)

    def test_forged_transport_or_payload_fields_cannot_enable_mutation_retry(self):
        value = observe("github", "registered/account", "mutation", now_ms=NOW,
                        transport="outcome-unknown")
        for update in ({"mutation_disposition": "known-throttle-refusal"},
                       {"response_class": "accepted"}, {"body": "private"}):
            forged = copy.deepcopy(value)
            forged.update(update)
            with self.subTest(update=update), self.assertRaises(ValueError):
                self.decision(forged)


class HTTPFixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path == "/timeout":
                    time.sleep(.15)
                status = {"/github": 403, "/kubernetes": 429, "/gone": 410,
                          "/server-error": 500, "/timeout": 200}[self.path]
                self.send_response(status)
                if self.path in {"/github", "/kubernetes"}:
                    self.send_header("Retry-After", "2")
                self.send_header("X-Private", "fixture-private-token")
                self.send_header("Content-Length", "30")
                self.end_headers()
                try:
                    self.wfile.write(b"person@example.invalid private")
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def log_message(self, *_args):
                pass

            def do_POST(self):
                self.do_GET()
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=2)

    def request(self, path, provider, operation="read", timeout=1):
        connection = http.client.HTTPConnection("127.0.0.1", self.server.server_port, timeout=timeout)
        try:
            connection.request("POST" if operation == "mutation" else "GET", path)
            response = connection.getresponse()
            # The actual collector may handle permitted payload separately. This
            # scheduling boundary never reads or persists the provider body.
            return observe(provider, "registered/loopback-fixture", operation,
                now_ms=NOW, http_status=response.status, headers=response.getheaders())
        except (socket.timeout, ConnectionError, OSError):
            return observe(provider, "registered/loopback-fixture", operation,
                now_ms=NOW, transport="outcome-unknown")
        finally:
            connection.close()

    def test_actual_http_rate_refusals_preserve_only_scheduling_fields(self):
        for path, provider in (("/github", "github"), ("/kubernetes", "kubernetes")):
            value = self.request(path, provider)
            self.assertEqual(value["response_class"], "rate-limited")
            self.assertNotIn("fixture-private-token", json.dumps(value))
            self.assertNotIn("person@example.invalid", json.dumps(value))
            self.assertEqual(schedule(value, now_ms=NOW, deadline_ms=NOW + 360000)["action"], "defer")

    def test_actual_http_gone_requires_relist_with_explicit_history_gap(self):
        value = self.request("/gone", "kubernetes", "watch")
        self.assertEqual(schedule(value, now_ms=NOW, deadline_ms=NOW + 10000)["action"], "relist")
        self.assertEqual(value["history_coverage"], "lost-interval")

    def test_actual_timeout_and_server_error_do_not_retry_unknown_mutation(self):
        for path in ("/timeout", "/server-error"):
            value = self.request(path, "github", "mutation", timeout=.02)
            result = schedule(value, now_ms=NOW, deadline_ms=NOW + 10000)
            self.assertEqual(result["action"], "reconcile")
            self.assertFalse(result["execution_permission"])


if __name__ == "__main__":
    unittest.main()
