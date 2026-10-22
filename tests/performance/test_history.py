"""Fault controls for independent identity and source-execution checking."""

import copy
from pathlib import Path
import tempfile
import unittest

from history import check_history
from paced import occupancy_summary, parameters, source_events


class HistoryTests(unittest.TestCase):
    def setUp(self):
        self.offers = [{"request_id": "a", "fault_mode": "success"},
                       {"request_id": "b", "fault_mode": "success"}]
        self.records = [{"request_id": key, "execution": "completed", "effect_id": f"effect-{key}"}
                        for key in ("a", "b")]
        self.source = [{"request_id": key, "effect_id": f"effect-{key}", "kind": kind, "at_ms": timestamp}
                       for key, start in (("a", 10), ("b", 15))
                       for kind, timestamp in (("started", start), ("completed", start + 10))]

    def test_two_identified_completions_pass(self):
        self.assertEqual(check_history(self.offers, self.records, self.source)["status"], "PASS")
        self.assertEqual(occupancy_summary(self.source)["peak_observed_source_occupancy"], 2)
        self.assertEqual(occupancy_summary(self.source)["source_occupancy_seconds"], .02)

    def test_host_completion_alone_does_not_establish_application_outcome(self):
        self.records[0]["application_check"] = "unavailable"
        result = check_history(self.offers, self.records, self.source)
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("application_outcome_mismatch:a", result["errors"])

    def test_duplicate_and_missing_result_with_equal_totals_fails(self):
        records = [self.records[0], copy.deepcopy(self.records[0])]
        result = check_history(self.offers, records, self.source)
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("result_identity_count:a:2", result["errors"])
        self.assertIn("result_identity_count:b:0", result["errors"])

    def test_duplicate_source_execution_even_with_same_effect_id_fails(self):
        result = check_history(self.offers, self.records, self.source + self.source[:2])
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("duplicate_source_execution:a", result["errors"])

    def test_missing_source_is_inconclusive(self):
        result = check_history(self.offers, self.records, self.source[2:])
        self.assertEqual(result["status"], "INCONCLUSIVE")
        self.assertIn("source_coverage_incomplete:a", result["coverage_gaps"])

    def test_lost_host_completion_preserves_external_fact_without_reexecution(self):
        self.offers[0]["fault_mode"] = "lost-completion"
        self.records[0].update(execution="unknown", effect_id=None, replay_source_unchanged=True)
        result = check_history(self.offers, self.records, self.source)
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(result["source_completed_with_host_unsettled"], 1)
        self.records[0]["replay_source_unchanged"] = False
        self.assertIn("replay_redispatched:a", check_history(self.offers, self.records, self.source)["errors"])

    def test_lost_reply_cannot_be_reported_as_host_completion(self):
        self.offers[0]["fault_mode"] = "lost-completion"
        result = check_history(self.offers, self.records, self.source)
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("lost_completion_claimed_completed:a", result["errors"])

    def test_before_effect_refusal_must_have_no_source_dispatch(self):
        self.offers[0]["fault_mode"] = "refuse-before-effect"
        self.records[0]["execution"] = "failed"
        self.assertEqual(check_history(self.offers, self.records, self.source[2:])["status"], "PASS")
        self.assertEqual(check_history(self.offers, self.records, self.source)["status"], "FAIL")

    def test_unknown_host_failure_does_not_establish_fixture_refusal(self):
        self.offers[0]["fault_mode"] = "refuse-before-effect"
        self.records[0]["execution"] = "unknown"
        result = check_history(self.offers, self.records, self.source[2:])
        self.assertEqual(result["status"], "INCONCLUSIVE")
        self.assertIn("refusal_not_observed:a", result["coverage_gaps"])

    def test_lost_reply_without_replay_check_is_inconclusive(self):
        self.offers[0]["fault_mode"] = "lost-completion"
        self.records[0]["execution"] = "unknown"
        result = check_history(self.offers, self.records, self.source)
        self.assertEqual(result["status"], "INCONCLUSIVE")
        self.assertIn("replay_source_check_unavailable:a", result["coverage_gaps"])

    def test_foreign_source_or_swapped_effect_identity_fails(self):
        source = copy.deepcopy(self.source)
        source[0]["effect_id"] = "another-effect"
        self.assertEqual(check_history(self.offers, self.records, source)["status"], "FAIL")
        source = self.source + [{"request_id": "foreign", "kind": "started"}]
        self.assertIn("unoffered_source_identity", check_history(self.offers, self.records, source)["errors"])

    def test_zero_width_source_interval_has_no_negative_occupancy(self):
        source = copy.deepcopy(self.source[:2])
        source[1]["at_ms"] = source[0]["at_ms"]
        result = occupancy_summary(source)
        self.assertEqual(result["source_occupancy_seconds"], 0)
        self.assertEqual(result["peak_observed_source_occupancy"], 0)


class PacedBoundsTests(unittest.TestCase):
    def test_declared_local_limits_reject_unbounded_or_nonfinite_load(self):
        for args in ((0, .25, 30000, 16, 0, "lost-completion"),
                     (1, float("nan"), 1, 1, 0, "lost-completion"),
                     (1, float("inf"), 1, 1, 0, "lost-completion"),
                     (1, 1, 30001, 1, 0, "lost-completion"),
                     (1, 1, 1, 17, 0, "lost-completion"),
                     (10000, .01, 30000, 1, 0, "lost-completion"),
                     (1, 1, 1, 1, 2, "lost-completion")):
            with self.subTest(args=args), self.assertRaises(ValueError):
                parameters(*args)

    def test_source_ledger_is_bounded_and_malformed_events_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source.jsonl"
            self.assertEqual(source_events(path), [])
            path.write_text("[]\n", encoding="utf-8")
            with self.assertRaises(RuntimeError):
                source_events(path)
            path.write_bytes(b"x" * 65537)
            with self.assertRaises(RuntimeError):
                source_events(path)


if __name__ == "__main__":
    unittest.main()
