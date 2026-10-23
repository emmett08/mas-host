"""Bounds and fault controls for the actual status-read capture consumer."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from capture import Limits, Monitor, PROFILE, check_capture, project_status
from coalescing import BASE, HTTPRPC, Reply
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tests/performance"))
from history import check_history

NOW = 1800000000000
TASK = "a2a-" + "a" * 56
TASK2 = "a2a-" + "b" * 56
KEY = b"metadata-capture-key-fixture-32bytes"
REG = {"id": TASK, "agent": "fixture-private-agent", "bundleDigest": "d" * 64}


def status(state="working", revision=1, **changes):
    value = {"version": "mas-host-task-status/1", **REG, "serviceState": state,
             "cancelRequested": False, "hostSettlement": "completed" if state == "completed" else "not-established",
             "externalExecution": "not-assessed", "payloadAvailability": "not-retained-in-service",
             "evidenceCoverage": "not-assessed", "applicationAssessment": "not-assessed",
             "sourceEvidenceFreshness": "not-assessed", "executionPermission": False,
             "captureProfile": PROFILE, "materialisation": {"revision": revision, "updatedAtMs": NOW,
                 "observedAtMs": NOW, "consistency": "committed-service-snapshot", "incomplete": False}}
    value.update(changes)
    return value


class RPC:
    authority = "registered-local-host"

    def __init__(self):
        self.value, self.calls, self.denied = status(), [], False

    def call(self, method, params, extensions, identity, **timing):
        self.calls.append((method, params, extensions))
        if self.denied:
            return Reply("rpc-unavailable")
        value = copy.deepcopy(self.value)
        value["id"] = params["id"]
        return Reply("ok", value)


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        os.chmod(self.directory.name, 0o700)
        self.path = Path(self.directory.name) / "capture.sqlite"
        self.rpc, self.monitors = RPC(), []
        self.now = NOW

    def tearDown(self):
        for monitor in self.monitors:
            monitor.close()
        self.directory.cleanup()

    def monitor(self, limits=Limits(pre_ms=1000, post_ms=1000, retention_ms=5000, sample_ms=100), registrations=None, key=KEY):
        monitor = Monitor(self.rpc, registrations or [REG], key=key, spool=self.path,
                          limits=limits, now_ms=NOW, clock=lambda: self.now)
        self.monitors.append(monitor)
        return monitor

    def poll(self, monitor, offset=0):
        self.now = NOW + offset
        return monitor.poll(now_ms=NOW + offset, deadline_ms=NOW + offset + 1000)

    def test_only_current_ordinary_source_profile_can_activate_capture(self):
        monitor = self.monitor()
        with self.assertRaises(ValueError):
            monitor.trigger(TASK, "operator", now_ms=NOW)
        self.rpc.value.pop("captureProfile")
        with self.assertRaises(ValueError):
            self.poll(monitor)
        self.assertEqual(monitor.summary(now_ms=NOW)["captures"], 0)
        self.rpc.value = status()
        self.rpc.value["materialisation"]["consistency"] = "committed-shared-service-snapshot"
        self.poll(monitor, 100)
        with self.assertRaises(ValueError):
            monitor.trigger(TASK, "operator", now_ms=NOW)

    def test_fixed_schema_rejects_payloads_and_identity_substitution(self):
        monitor = self.monitor()
        for index, changes in enumerate(({"output": "secret"}, {"agent": "another-agent"},
                        {"bundleDigest": "e" * 64}, {"executionPermission": True},
                        {"externalExecution": "completed"})):
            self.rpc.value = status(**changes)
            self.poll(monitor, index * 100)
            self.assertFalse(monitor.buffer)
        self.assertEqual(monitor.loss["sample_failed"], 5)

    def test_pre_post_projection_preserves_uncertainty_and_no_plain_identities(self):
        monitor = self.monitor()
        self.poll(monitor)
        self.rpc.value = status("unknown", 2)
        self.poll(monitor, 1000)
        reference = next(iter(monitor.cases))
        self.rpc.value = status("completed", 3)
        self.poll(monitor, 2000)
        view = monitor.read(reference, now_ms=NOW + 2001)
        self.assertEqual([record["state"] for record in view["records"]], ["working", "unknown", "completed"])
        self.assertTrue(view["closed"])
        self.assertEqual(check_capture(view)["status"], "INCONCLUSIVE")
        self.assertEqual({call[0] for call in self.rpc.calls}, {"GetTaskStatus"})
        self.assertTrue(all(call[2] == (BASE,) for call in self.rpc.calls))
        raw = self.path.read_bytes()
        self.assertNotIn(TASK.encode(), raw)
        self.assertNotIn(REG["agent"].encode(), raw)
        self.assertNotIn(KEY, raw)

    def test_trigger_storm_coalesces_without_extending_windows_and_capacity_is_hard(self):
        monitor = self.monitor(Limits(pre_ms=0, post_ms=1000, retention_ms=5000, max_captures=1))
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        for offset in range(1, 100):
            self.assertEqual(monitor.trigger(TASK, "operator", now_ms=NOW + offset), reference)
        view = monitor.read(reference, now_ms=NOW + 100)
        self.assertEqual(view["occurrences"], 100)
        self.assertEqual(view["end_ms"], NOW + 1000)
        self.assertIsNone(monitor.trigger(TASK, "revision-gap", now_ms=NOW + 100))
        self.assertEqual(monitor.loss["capture_refused"], 1)
        self.assertEqual(len(monitor.cases), 1)

    def test_byte_overflow_and_buffer_eviction_are_counted_with_bounded_storage(self):
        monitor = self.monitor(Limits(pre_ms=60000, post_ms=60000, retention_ms=70000,
                                     max_bytes=4096, buffer_bytes=1024, sample_ms=100))
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        for index in range(1, 90):
            self.rpc.value = status("working", index + 1)
            self.poll(monitor, index * 100)
        value = monitor.read(reference, now_ms=NOW + 9000)
        self.assertLessEqual(len(json.dumps(value, separators=(",", ":")).encode()), 4096)
        self.assertGreater(value["loss"]["byte_budget"], 0)
        self.assertGreater(value["loss"]["buffer_evicted"], 0)
        self.assertIn("byte-budget", check_capture(value)["coverage_gaps"])

    def test_revision_gaps_faulty_revisions_and_lost_polls_remain_visible(self):
        monitor = self.monitor()
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        self.rpc.value = status("working", 4)
        self.poll(monitor, 100)
        self.rpc.denied = True
        self.poll(monitor, 200)
        self.rpc.denied = False
        self.rpc.value = status("completed", 4)  # Same revision, changed committed state.
        self.poll(monitor, 300)
        result = check_capture(monitor.read(reference, now_ms=NOW + 300))
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("status-revision-conflict", result["errors"])
        self.assertIn("revision-gap", result["coverage_gaps"])
        self.assertIn("sample-failed", result["coverage_gaps"])

    def test_expiry_deletes_rows_and_old_reference_never_reads(self):
        monitor = self.monitor()
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        with self.assertRaises(ValueError):
            monitor.read(reference, now_ms=NOW + 5000)
        self.assertEqual(monitor.db.execute("SELECT count(*) FROM captures").fetchone()[0], 0)

    def test_restart_preserves_completed_metadata_and_marks_open_window_incomplete(self):
        monitor = self.monitor()
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        monitor.close()
        self.monitors.remove(monitor)
        reopened = self.monitor()
        value = reopened.read(reference, now_ms=NOW)
        self.assertTrue(value["closed"])
        self.assertIn("restart-interrupted", value["coverage_gaps"])
        self.assertEqual(value["loss"]["restart_interrupted"], 1)
        with self.assertRaises(ValueError):
            self.monitor(key=b"different-metadata-key-32bytes0000")

    def test_scope_key_and_row_tampering_fail_closed(self):
        monitor = self.monitor()
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        monitor.close()
        self.monitors.remove(monitor)
        with sqlite3.connect(self.path) as db:
            db.execute("UPDATE captures SET body='{}' WHERE id=?", (reference,))
        with self.assertRaises(ValueError):
            self.monitor()

    def test_invalid_limits_and_backwards_clock_are_rejected(self):
        for limits in (Limits(max_captures=17), Limits(max_bytes=65537), Limits(pre_ms=60001),
                       Limits(post_ms=60001), Limits(retention_ms=1000), Limits(sample_ms=99)):
            with self.assertRaises(ValueError):
                limits.validate()
        monitor = self.monitor()
        self.poll(monitor, 100)
        with self.assertRaises(ValueError):
            self.poll(monitor, 99)

    def test_shared_token_scope_never_creates_cross_authority_correlation(self):
        first = self.monitor()
        token = first.token("task", TASK)
        self.rpc.authority = "other-local-host"
        with self.assertRaises(ValueError):
            self.poll(first)
        self.assertEqual(token, first.token("task", TASK))

    def test_overlapping_spool_owners_cannot_bypass_capture_capacity(self):
        self.monitor()
        with self.assertRaises(ValueError):
            self.monitor()

    def test_spool_uri_metacharacters_do_not_change_the_locked_file(self):
        self.path = Path(self.directory.name) / "capture?other#name.sqlite"
        monitor = self.monitor()
        self.poll(monitor)
        monitor.trigger(TASK, "operator", now_ms=NOW)
        actual = monitor.db.execute("PRAGMA database_list").fetchone()[2]
        self.assertEqual(Path(actual), self.path)

    def test_late_response_does_not_enter_an_expired_post_window(self):
        monitor = self.monitor()
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        self.now = NOW + 2000
        monitor.poll(now_ms=NOW + 100, deadline_ms=NOW + 3000)
        value = monitor.read(reference, now_ms=monitor.clock)
        self.assertTrue(value["closed"])
        self.assertEqual(len(value["records"]), 1)
        self.assertIn("post-window-incomplete", value["coverage_gaps"])

    def test_status_refreshes_are_bounded_by_sampling_interval(self):
        monitor = self.monitor()
        self.poll(monitor)
        for offset in range(1, 100):
            self.poll(monitor, offset)
        self.assertEqual(len(self.rpc.calls), 1)
        self.poll(monitor, 100)
        self.assertEqual(len(self.rpc.calls), 2)

    def test_empty_zero_width_window_still_reports_missing_observation(self):
        monitor = self.monitor(Limits(pre_ms=0, post_ms=0, retention_ms=1000, sample_ms=100))
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW + 100)
        value = monitor.read(reference, now_ms=NOW + 101)
        self.assertEqual(value["records"], [])
        self.assertIn("pre-window-incomplete", check_capture(value)["coverage_gaps"])

    def test_checker_rejects_unclassified_extra_fields(self):
        monitor = self.monitor()
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        value = monitor.read(reference, now_ms=NOW)
        value["records"][0]["output"] = "private-body"
        with self.assertRaises(ValueError):
            check_capture(value)

    def test_capture_checker_supplements_independent_history_without_fabricating_source_events(self):
        monitor = self.monitor(Limits(pre_ms=0, post_ms=0, retention_ms=1000, sample_ms=100))
        self.poll(monitor)
        reference = monitor.trigger(TASK, "operator", now_ms=NOW)
        value = monitor.read(reference, now_ms=NOW + 1)
        offers = [{"request_id": "request-1"}]
        results = [{"request_id": "request-1", "execution": "completed", "effect_id": "effect-1"}]
        events = [{"request_id": "request-1", "kind": kind, "effect_id": "effect-1", "at_ms": at}
                  for kind, at in (("started", 1), ("completed", 2))]
        checked = check_history(offers, results, events, status_capture=value)
        self.assertEqual(checked["status"], "PASS")
        self.assertEqual(checked["status_capture"]["status"], "INCONCLUSIVE")
        self.assertEqual(check_history(offers, results, [], status_capture=value)["status"], "INCONCLUSIVE")
        changed = copy.deepcopy(value["records"][0])
        changed.update(state="completed", host_settlement="completed")
        value["records"].append(changed)
        checked = check_history(offers, results, events, status_capture=value)
        self.assertEqual(checked["status"], "FAIL")
        self.assertIn("capture:status-revision-conflict", checked["errors"])

    def test_actual_http_uses_current_bearer_and_only_status_method(self):
        requests = []

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                requests.append((request, dict(self.headers)))
                body = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": status("unknown")}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            token = ["first-token"]
            self.rpc = HTTPRPC(f"http://127.0.0.1:{server.server_port}/a2a", "local-host", "status-reader",
                               lambda: token[0], allow_loopback_http=True, clock=lambda: NOW)
            monitor = self.monitor()
            self.poll(monitor)
            token[0] = "rotated-token"
            self.poll(monitor, 100)
            self.assertEqual([r[0]["method"] for r in requests], ["GetTaskStatus", "GetTaskStatus"])
            self.assertEqual([r[1]["Authorization"] for r in requests], ["Bearer first-token", "Bearer rotated-token"])
            self.assertTrue(all(r[1]["A2A-Version"] == "1.0" for r in requests))
            self.assertTrue(all(r[1]["A2A-Extensions"] == BASE for r in requests))
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    unittest.main()
