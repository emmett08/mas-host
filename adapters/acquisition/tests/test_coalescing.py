"""Exact host protocol identities, caller isolation and bounded real HTTP."""

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
from coalescing import (Acquisition, BASE, HTTPRPC, Publication, PUBLICATION,
                        RegisteredRead, Reply, SHARED_READ, Subscription, _decode)
from provider import observe, schedule


NOW = 1800000000000
TASK = "a2a-" + "a" * 56
REF = "shared-" + "b" * 64
PRIVATE = "fixture-private-artifact-and-token"
IDENTITY_KEY = b"synthetic-identity-key-0123456789012345678901234567"


def registered(spec, value):
    return RegisteredRead(spec, value, identity_key=IDENTITY_KEY)


def contract(**changes):
    value = {"version": "mas-coalesced-read/1", "authority": "fixture-host/1",
        "tenant": "tenant-a", "acquisition_grant": "grant-producer",
        "purpose": "diagnosis", "credential_epoch": "credential-1", "policy_epoch": "policy-1",
        "source": "registered-kubernetes", "query": {"namespace_uid": "namespace-uid-1", "kind": "Deployment"},
        "generation": "generation-1", "projection": "deployment-point/1",
        "schema_version": "point/1", "transform_version": "collector/1", "agent": "collector",
        "descriptor_digest": "d" * 64, "effect_key": "acquire", "submission_deadline_ms": NOW + 10000,
        "transport_quota_scope": "registered-host/task-service"}
    value.update(changes)
    return value


def status(state="completed", **changes):
    value = {"version": "mas-host-task-status/1", "id": TASK, "agent": "collector",
        "bundleDigest": "d" * 64, "serviceState": state,
        "hostSettlement": "completed" if state == "completed" else "not-established",
        "executionPermission": False}
    value.update(changes)
    return value


def source(**changes):
    value = {"version": "mas-governed-source-view/1", "actor": "collector", "key": "acquire",
        "host_settlement": "completed", "payload_disposition": "available",
        "source_id": "source-original", "effect_id": "effect-original",
        "source_metadata": {"id": "source-original", "step_id": "effect-original",
                            "acquired_at_ms": NOW - 100, "expires_at_ms": NOW + 10000},
        "projection": {"ready_replicas": 2}, "lineage": [{"id": "original-source"}]}
    value.update(changes)
    return value


class FakeRPC:
    authority = "fixture-host/1"

    def __init__(self, *, allowed=True):
        self.calls = []
        self.allowed = allowed
        self.submissions = {}
        self.physical_executions = 0
        self.lost = set()
        self.task_status = status()
        self.view = source()
        self.hook = None

    def call(self, method, params, extensions, request_id, **timing):
        self.calls.append((method, copy.deepcopy(params), extensions, request_id, timing))
        if self.hook:
            self.hook(method)
        if method == "SendMessage":
            self.assert_submission(params)
            original = params["message"]["messageId"]
            if original in self.submissions:
                if self.submissions[original] != params:
                    raise AssertionError("changed replay")
            else:
                self.submissions[original] = copy.deepcopy(params)
                self.physical_executions += 1
            reply = Reply("ok", {"task": {"id": TASK, "artifacts": [PRIVATE]}})
        elif method == "GetTaskStatus":
            reply = Reply("ok", self.task_status)
        elif method == "PublishRetainedSource":
            reply = Reply("ok", {"version": "mas-governed-source-publication/1",
                                  "status": "published", "reference": REF})
        elif method == "GetSharedSource":
            reply = Reply("ok", self.view) if self.allowed else Reply("rpc-unavailable")
        else:
            raise AssertionError("unexpected or upstream cancellation method")
        if method in self.lost:
            self.lost.remove(method)
            return Reply("transport-unknown")
        return reply

    @staticmethod
    def assert_submission(params):
        assert params["configuration"] == {"returnImmediately": True}


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.registration = registered(contract(), {"mode": "acquire", "quantity": 2})
        self.producer = FakeRPC()

    def acquisition(self, **kwargs):
        return Acquisition(self.registration, self.producer, **kwargs)

    def publish(self):
        acquisition = self.acquisition()
        result = acquisition.advance(now_ms=NOW, resolution_deadline_ms=NOW + 10000)
        self.assertEqual(result["state"], "published")
        return acquisition, result["publication"]

    def subscription(self, rpc=None, **kwargs):
        return Subscription("subscriber-1", self.registration, rpc or FakeRPC(),
            deadline_ms=NOW + 10000, clock=lambda: NOW, **kwargs)

    def test_lost_submission_ack_reuses_exact_original_host_identity(self):
        self.producer.lost.add("SendMessage")
        acquisition = self.acquisition()
        first = acquisition.advance(now_ms=NOW, resolution_deadline_ms=NOW + 10000)
        self.assertEqual(first["state"], "submission-unresolved")
        resumed = self.acquisition(trusted_checkpoint=acquisition.checkpoint())
        result = resumed.advance(now_ms=NOW + 1, resolution_deadline_ms=NOW + 10000)
        self.assertEqual(result["state"], "published")
        submissions = [entry for entry in self.producer.calls if entry[0] == "SendMessage"]
        self.assertEqual(submissions[0][1:4], submissions[1][1:4])
        self.assertEqual(self.producer.physical_executions, 1)
        self.assertNotIn(PRIVATE, repr(result))
        self.assertNotIn(TASK, repr(result))

    def test_lost_publication_ack_replays_same_alias_without_new_submission(self):
        self.producer.lost.add("PublishRetainedSource")
        acquisition = self.acquisition()
        self.assertEqual(acquisition.advance(now_ms=NOW,
            resolution_deadline_ms=NOW + 10000)["state"], "publication-unresolved")
        resumed = self.acquisition(trusted_checkpoint=acquisition.checkpoint())
        self.assertEqual(resumed.advance(now_ms=NOW + 1,
            resolution_deadline_ms=NOW + 10000)["state"], "published")
        calls = [c for c in self.producer.calls if c[0] == "PublishRetainedSource"]
        self.assertEqual(calls[0][1:4], calls[1][1:4])
        self.assertEqual(len([c for c in self.producer.calls if c[0] == "SendMessage"]), 1)

    def test_exact_key_binds_scope_generation_policy_epochs_and_input(self):
        for field in ("tenant", "acquisition_grant", "purpose", "credential_epoch", "policy_epoch",
                      "source", "generation", "projection", "schema_version", "transform_version", "authority"):
            changed = registered(contract(**{field: "different"}), {"mode": "acquire", "quantity": 2})
            self.assertNotEqual(changed.query_id, self.registration.query_id, field)
            self.assertNotEqual(changed.publication_alias, self.registration.publication_alias, field)
        changed = registered(contract(), {"mode": "acquire", "quantity": 3})
        self.assertNotEqual(changed.query_id, self.registration.query_id)
        self.assertEqual(registered(contract(), {"quantity": 2, "mode": "acquire"}).query_id,
                         self.registration.query_id)

    def test_caller_mutations_do_not_change_frozen_replay_content(self):
        original = {"mode": "acquire", "items": [1]}
        spec = contract()
        frozen = registered(spec, original)
        old = frozen.submission()
        original["items"].append(2)
        spec["query"]["namespace_uid"] = "different"
        frozen.contract()["agent"] = "different"
        self.assertEqual(frozen.submission(), old)

    def test_follower_cannot_extend_upstream_submission_cutoff(self):
        result = self.acquisition().advance(now_ms=NOW + 10000,
            resolution_deadline_ms=NOW + 1000000)
        self.assertEqual(result["state"], "submission-expired-unresolved")
        self.assertEqual(self.producer.calls, [])

    def test_known_task_can_resolve_after_submission_cutoff_without_new_acquisition(self):
        self.producer.task_status = status("working")
        acquisition = self.acquisition()
        self.assertEqual(acquisition.advance(now_ms=NOW,
            resolution_deadline_ms=NOW + 10000)["state"], "pending")
        resumed = self.acquisition(trusted_checkpoint=acquisition.checkpoint())
        self.producer.task_status = status()
        self.assertEqual(resumed.advance(now_ms=NOW + 20000,
            resolution_deadline_ms=NOW + 30000)["state"], "published")
        self.assertEqual(len([c for c in self.producer.calls if c[0] == "SendMessage"]), 1)

    def test_failed_unknown_or_changed_identity_never_publishes(self):
        for value in (status("failed"), status("unknown"), status(agent="other"),
                      status(bundleDigest="e" * 64), status(executionPermission=True)):
            self.producer.task_status = value
            acquisition = self.acquisition()
            result = acquisition.advance(now_ms=NOW, resolution_deadline_ms=NOW + 10000)
            self.assertNotEqual(result["state"], "published")
        self.assertNotIn("PublishRetainedSource", [c[0] for c in self.producer.calls])

    def test_private_checkpoint_requires_exact_registration_binding(self):
        acquisition, _ = self.publish()
        checkpoint = acquisition.checkpoint()
        self.assertIn(TASK, repr(checkpoint))
        checkpoint["query_id"] = "0" * 64
        with self.assertRaises(ValueError):
            self.acquisition(trusted_checkpoint=checkpoint)
        checkpoint["query_id"] = self.registration.query_id
        checkpoint["bearer"] = PRIVATE
        with self.assertRaises(ValueError):
            self.acquisition(trusted_checkpoint=checkpoint)

    def test_private_identity_key_hides_dictionary_inputs_and_rotation_cannot_restore_old_checkpoint(self):
        acquisition, _ = self.publish()
        changed_key = RegisteredRead(contract(), {"mode": "acquire", "quantity": 2}, identity_key=b"x" * 32)
        self.assertNotEqual(changed_key.query_id, self.registration.query_id)
        with self.assertRaises(ValueError):
            Acquisition(changed_key, self.producer, trusted_checkpoint=acquisition.checkpoint())
        for key in (b"public-salt", "private-key", bytearray(b"x" * 32)):
            with self.assertRaises(ValueError):
                RegisteredRead(contract(), {}, identity_key=key)

    def test_subscribers_use_independent_authority_and_cancel_only_own_delivery(self):
        _, publication = self.publish()
        permitted, denied = FakeRPC(), FakeRPC(allowed=False)
        allowed_sub = self.subscription(permitted)
        denied_sub = self.subscription(denied)
        denied_sub.cancel()
        self.assertEqual(denied_sub.deliver(publication, now_ms=NOW)["state"], "cancelled")
        self.assertEqual(denied.calls, [])
        result = allowed_sub.deliver(publication, now_ms=NOW)
        self.assertEqual(result["state"], "available")
        self.assertEqual(result["source"]["source_metadata"]["acquired_at_ms"], NOW - 100)
        self.assertEqual(result["source"]["lineage"], [{"id": "original-source"}])
        denied_sub = self.subscription(denied)
        self.assertEqual(denied_sub.deliver(publication, now_ms=NOW)["state"], "unavailable")
        self.assertEqual(permitted.calls[0][0:3], ("GetSharedSource", {"reference": REF}, (BASE, SHARED_READ)))
        self.assertEqual(self.producer.physical_executions, 1)

    def test_cancellation_or_deadline_during_http_read_discards_own_delivery(self):
        _, publication = self.publish()
        rpc = FakeRPC()
        sub = self.subscription(rpc)
        rpc.hook = lambda _: sub.cancel()
        self.assertEqual(sub.deliver(publication, now_ms=NOW)["state"], "cancelled")
        self.assertEqual([c[0] for c in rpc.calls], ["GetSharedSource"])
        sub = Subscription("subscriber-late", self.registration, FakeRPC(),
            deadline_ms=NOW + 10, clock=lambda: NOW + 10)
        self.assertEqual(sub.deliver(publication, now_ms=NOW)["state"], "deadline-expired")

    def test_expired_or_stale_source_is_never_refetched_or_timestamp_renewed(self):
        _, publication = self.publish()
        for kwargs in ({"min_acquired_at_ms": NOW}, {"max_age_ms": 50}):
            result = self.subscription(**kwargs).deliver(publication, now_ms=NOW)
            self.assertEqual(result["state"], "stale")
            self.assertEqual(result["acquired_at_ms"], NOW - 100)
            self.assertFalse(result["fresh_acquisition_started"])
        rpc = FakeRPC()
        rpc.view["source_metadata"]["expires_at_ms"] = NOW
        self.assertEqual(self.subscription(rpc).deliver(publication, now_ms=NOW)["state"], "stale")
        self.assertEqual([c[0] for c in rpc.calls], ["GetSharedSource"])

    def test_withheld_invalid_or_unbound_source_never_delivers_projection(self):
        _, publication = self.publish()
        for update in ({"payload_disposition": "withheld"}, {"actor": "other"},
                       {"key": "other"}, {"host_settlement": "unknown"},
                       {"source_metadata": {"acquired_at_ms": True}}):
            rpc = FakeRPC()
            rpc.view.update(update)
            result = self.subscription(rpc).deliver(publication, now_ms=NOW)
            self.assertNotEqual(result["state"], "available")
            self.assertNotIn("source", result)
        with self.assertRaises(ValueError):
            self.subscription().deliver(Publication("other", self.registration.query_id, REF), now_ms=NOW)

    def test_registration_rejects_floats_duplicates_unbounded_or_secret_bearing_unknown_fields(self):
        for update in ({"extra": PRIVATE}, {"submission_deadline_ms": True}, {"descriptor_digest": 3}):
            with self.subTest(update=update), self.assertRaises(ValueError):
                registered(contract(**update), {})
        for value in (1.2, {"huge": "x" * 65536}, {"not": object()}):
            with self.assertRaises(ValueError):
                registered(contract(), value)
        with self.assertRaises(ValueError):
            _decode(b'{"query":1,"query":2}')


class HTTPTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.requests = []
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                body = self.rfile.read(int(self.headers["Content-Length"]))
                HTTPTests.requests.append((self.path, self.headers.get("Authorization"),
                    self.headers.get("A2A-Extensions"), json.loads(body), self.headers.get("A2A-Version")))
                status_code = {"/redirect": 302, "/throttle": 429, "/private-error": 403}.get(self.path, 200)
                self.send_response(status_code)
                self.send_header("Content-Type", "application/json")
                self.send_header("X-Private", PRIVATE)
                if status_code == 302:
                    self.send_header("Location", "/followed-forbidden")
                if status_code == 429:
                    self.send_header("Retry-After", "2")
                self.end_headers()
                message = {"jsonrpc": "2.0", "id": json.loads(body)["id"], "result": source()}
                if self.path == "/rpc-error":
                    message = {"jsonrpc": "2.0", "id": json.loads(body)["id"],
                               "error": {"message": PRIVATE, "data": PRIVATE}}
                if self.path == "/wrong-id":
                    message["id"] = "wrong"
                data = (PRIVATE.encode() if status_code != 200 else
                        b"x" * 65537 if self.path == "/oversize" else json.dumps(message).encode())
                try:
                    if self.path == "/drip":
                        for byte in data:
                            self.wfile.write(bytes([byte]))
                            self.wfile.flush()
                            time.sleep(.01)
                    else:
                        self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError, socket.error):
                    pass
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

    def rpc(self, path="/rpc", bearer=lambda: PRIVATE, **kwargs):
        return HTTPRPC(f"http://127.0.0.1:{self.server.server_port}{path}", "fixture-host/1",
            "registered-host/task-service", bearer, allow_loopback_http=True, clock=lambda: NOW, **kwargs)

    def read(self, rpc, method="GetSharedSource"):
        return rpc.call(method, {"reference": REF},
                        (BASE, SHARED_READ) if method == "GetSharedSource" else (BASE,), "subscriber-http",
                        now_ms=NOW, deadline_ms=NOW + 10000)

    def test_actual_http_uses_current_separate_bearer_and_exact_extensions(self):
        token = ["producer-token"]
        rpc = self.rpc(bearer=lambda: token[0])
        self.assertEqual(self.read(rpc).state, "ok")
        token[0] = "subscriber-token"
        self.assertEqual(self.read(rpc).state, "ok")
        self.assertEqual(self.requests[-2][1], "Bearer producer-token")
        self.assertEqual(self.requests[-1][1], "Bearer subscriber-token")
        self.assertEqual(self.requests[-1][2], ", ".join((BASE, SHARED_READ)))
        self.assertEqual(self.requests[-1][4], "1.0")
        self.assertNotIn(PRIVATE, repr(rpc))

    def test_actual_http_suppresses_errors_and_never_follows_redirects(self):
        before = len(self.requests)
        for path in ("/private-error", "/rpc-error", "/redirect"):
            reply = self.read(self.rpc(path))
            self.assertNotEqual(reply.state, "ok")
            self.assertNotIn(PRIVATE, repr(reply) + json.dumps(reply.safe()))
        self.assertEqual(len(self.requests), before + 3)
        self.assertNotIn("/followed-forbidden", [r[0] for r in self.requests])

    def test_actual_host_throttle_gates_read_but_never_claims_external_quota(self):
        rpc = self.rpc("/throttle")
        reply = self.read(rpc)
        self.assertEqual(reply.state, "http-defer")
        self.assertEqual(reply.observation["provider"], "a2a")
        self.assertEqual(reply.decision["not_before_ms"], NOW + 2000)
        before = len(self.requests)
        repeated = self.read(rpc)
        self.assertEqual(repeated.state, "locally-defer")
        self.assertEqual(len(self.requests), before)
        self.assertEqual(repeated.decision["not_before_ms"], NOW + 2000)
        mutation = self.read(self.rpc("/throttle"), method="SendMessage")
        self.assertEqual(mutation.state, "http-reconcile")
        self.assertFalse(mutation.decision["execution_permission"])

    def test_actual_oversize_mismatched_reply_or_dripping_server_cannot_deliver(self):
        for path in ("/oversize", "/wrong-id", "/drip"):
            start = time.monotonic()
            reply = self.read(self.rpc(path, timeout_ms=60))
            self.assertEqual(reply.state, "transport-unknown")
            self.assertLess(time.monotonic() - start, 1)
            self.assertNotIn(PRIVATE, json.dumps(reply.safe()))

    def test_endpoint_or_bearer_cannot_redirect_or_inject_headers(self):
        for endpoint in ("http://example.invalid/rpc", "http://127.0.0.1/rpc?token=private",
                         "https://user:private@example.invalid/rpc", "http://127.0.0.1/rpc\r\nprivate"):
            with self.assertRaises(ValueError):
                HTTPRPC(endpoint, "host/1", "host/quota", lambda: PRIVATE, allow_loopback_http=True)
        before = len(self.requests)
        self.assertEqual(self.read(self.rpc(bearer=lambda: "private\r\nX: private")).state,
                         "transport-unknown")
        self.assertEqual(len(self.requests), before)


class GenericHostProviderTests(unittest.TestCase):
    def test_host_429_never_inherits_github_reset_or_mutation_refusal_contract(self):
        value = observe("a2a", "host/quota", "mutation", now_ms=NOW, http_status=429,
            headers=[("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", "1")])
        self.assertIsNone(value["rate_limit_reset_ms"])
        self.assertEqual(value["mutation_disposition"], "unknown")
        self.assertEqual(schedule(value, now_ms=NOW, deadline_ms=NOW + 10000)["action"], "reconcile")


if __name__ == "__main__":
    unittest.main()
