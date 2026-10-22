"""Actual bounded GETs through immutable credential-free source registrations."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import socket
import sys
import threading
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from read_worker import PublicGET

NOW = 1800000000000
PRIVATE = "fixture-private-person@example.invalid"


class ReadTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.calls = []
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                ReadTests.calls.append((self.path, dict(self.headers)))
                status = {"/throttle": 429, "/gone": 410, "/redirect": 302,
                          "/conditional": 304, "/refused": 403}.get(self.path, 200)
                self.send_response(status)
                self.send_header("X-Private", PRIVATE)
                if status == 429:
                    self.send_header("Retry-After", "2")
                if status == 302:
                    self.send_header("Location", "/forbidden-follow")
                self.end_headers()
                raw = (b"x" * 32769 if self.path == "/oversize" else b'{"ok":true,"ok":false}'
                       if self.path == "/duplicate" else json.dumps({"status": {"ready": 2},
                           "metadata": {"name": "synthetic"}, "private": PRIVATE}).encode())
                try:
                    if self.path == "/drip":
                        for byte in raw:
                            self.wfile.write(bytes([byte]))
                            self.wfile.flush()
                            time.sleep(.01)
                    else:
                        self.wfile.write(raw)
                except (BrokenPipeError, ConnectionResetError, socket.error):
                    pass
            def do_POST(self):
                raise AssertionError("mutation is forbidden")
            def log_message(self, *_args):
                pass
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=2)

    def contract(self, path="/read", **changes):
        value = {"version": "mas-provider-public-get/1", "provider": "github",
            "source_ref": "fixture/public-repository", "quota_scope": "github/public-fixture",
            "endpoint": f"http://127.0.0.1:{self.server.server_port}{path}", "operation": "read",
            "if_none_match": None, "allow_loopback_http": True, "timeout_ms": 1000,
            "max_body_bytes": 32768,
            "projection": {"ready": {"pointer": "/status/ready", "type": "integer"},
                           "name": {"pointer": "/metadata/name", "type": "string"}}}
        value.update(changes)
        return value

    def read(self, contract=None, **kwargs):
        return PublicGET(contract or self.contract()).read(deadline_ms=NOW + 10000,
                                                          clock=lambda: NOW, **kwargs)

    def test_actual_read_retains_only_approved_structured_projection(self):
        result = self.read()
        self.assertEqual(result["payload_disposition"], "available")
        self.assertEqual(result["projection"], {"ready": 2, "name": "synthetic"})
        self.assertEqual(result["provider_observation"]["http_status"], 200)
        self.assertFalse(result["provider_observation"]["body_retained"])
        self.assertNotIn(PRIVATE, json.dumps(result))
        self.assertNotIn("Authorization", self.calls[-1][1])

    def test_registered_contract_is_snapshotted_and_input_has_no_endpoint_seam(self):
        contract = self.contract()
        read = PublicGET(contract)
        contract["endpoint"] += "/changed"
        contract["projection"]["ready"]["pointer"] = "/private"
        self.assertEqual(read.read(deadline_ms=NOW + 10000, clock=lambda: NOW)["projection"]["ready"], 2)
        self.assertNotIn("127.0.0.1", repr(read))

    def test_conditional_get_keeps_not_modified_distinct_from_fresh_evidence(self):
        result = self.read(self.contract("/conditional", if_none_match='"original-etag"'))
        self.assertEqual(result["payload_disposition"], "not-modified")
        self.assertIsNone(result["projection"])
        self.assertEqual(self.calls[-1][1]["If-None-Match"], '"original-etag"')
        self.assertEqual(result["provider_observation"]["response_class"], "not-modified")

    def test_throttle_keeps_secondary_floor_without_http_retry(self):
        before = len(self.calls)
        result = self.read(self.contract("/throttle"))
        self.assertEqual(len(self.calls), before + 1)
        observation = result["provider_observation"]
        self.assertEqual(observation["retry_after_not_before_ms"], NOW + 2000)
        self.assertEqual(observation["retry_not_before_ms"], NOW + 60000)
        self.assertEqual(result["provider_schedule"]["action"], "expired")
        self.assertIsNone(result["projection"])

    def test_kubernetes_gone_relist_preserves_lost_interval(self):
        result = self.read(self.contract("/gone", provider="kubernetes", operation="watch"))
        self.assertEqual(result["provider_observation"]["history_coverage"], "lost-interval")
        self.assertEqual(result["provider_schedule"]["action"], "relist")
        self.assertFalse(result["provider_schedule"]["new_snapshot_repairs_history"])

    def test_refusal_or_redirect_retains_no_body_or_followup(self):
        before = len(self.calls)
        for path in ("/refused", "/redirect"):
            result = self.read(self.contract(path))
            self.assertEqual(result["payload_disposition"], "unavailable")
            self.assertEqual(result["provider_schedule"]["action"], "refused")
            self.assertNotIn(PRIVATE, json.dumps(result))
        self.assertEqual(len(self.calls), before + 2)
        self.assertNotIn("/forbidden-follow", [call[0] for call in self.calls])

    def test_oversized_duplicate_or_slow_body_cannot_deliver_or_retry(self):
        before = len(self.calls)
        for path in ("/oversize", "/duplicate", "/drip"):
            started = time.monotonic()
            result = self.read(self.contract(path, timeout_ms=60))
            self.assertEqual(result["payload_disposition"], "unavailable")
            self.assertEqual(result["response_http_status"], 200)
            self.assertLess(time.monotonic() - started, 1)
            self.assertNotIn(PRIVATE, json.dumps(result))
        self.assertEqual(len(self.calls), before + 3)

    def test_deadline_expiry_prevents_any_network_request(self):
        before = len(self.calls)
        result = PublicGET(self.contract()).read(deadline_ms=NOW, clock=lambda: NOW)
        self.assertFalse(result["request_dispatched"])
        self.assertEqual(result["provider_observation"]["transport"], "not-dispatched")
        self.assertEqual(len(self.calls), before)

    def test_unregistered_authentication_endpoints_and_mutations_are_rejected(self):
        invalid = [{"operation": "mutation"}, {"bearer": PRIVATE}, {"allow_loopback_http": False},
                   {"endpoint": "https://user:private@api.github.com/repos/a/b"},
                   {"endpoint": "https://enterprise.invalid/repos/a/b"},
                   {"endpoint": "http://169.254.169.254/latest/meta-data"},
                   {"endpoint": "https://api.github.com/private", "provider": "kubernetes"},
                   {"if_none_match": '"etag"\r\nAuthorization: private'}, {"max_body_bytes": 32769}]
        for changes in invalid:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                PublicGET(self.contract(**changes))


if __name__ == "__main__":
    unittest.main()
