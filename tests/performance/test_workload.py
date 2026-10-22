"""Counterexamples for fixture identity, encoded bounds and outcome projection."""

import copy
import unittest

from baseline import classify, latency_summary, totals
from workload import encoded, generate, load_manifest


class WorkloadTests(unittest.TestCase):
    def test_canonical_manifest_keeps_unmeasured_scenarios_unrun(self):
        manifest = load_manifest()
        self.assertEqual(set(manifest["scenarios"]), {"V", "L", "S", "A", "A+", "E", "E+"})
        self.assertTrue(all(s["status"] == "NOT_RUN" and s["missing"]
                            for s in manifest["scenarios"].values()))
        self.assertIsNone(manifest["reference_hardware"])
        self.assertIsNone(manifest["cost_ceiling"])

    def test_generation_preserves_prefixes_and_binds_workload_parameters(self):
        records = generate(20)
        self.assertEqual(records, generate(20))
        self.assertEqual(records[:3], generate(3))
        self.assertEqual(len({r["request_id"] for r in records}), 20)
        for kwargs in ({"seed": 3}, {"distribution": "skewed-80-percent"},
                       {"tenants": 3}, {"clusters": 4}, {"payload_bytes": 600}):
            self.assertNotEqual(records[0]["request_id"], generate(1, **kwargs)[0]["request_id"])

    def test_skew_is_exact_and_nonhot_tenants_stay_distinct(self):
        records = generate(100, distribution="skewed-80-percent", tenants=5)
        self.assertEqual(sum(r["tenant"] == "tenant-0000" for r in records), 80)
        self.assertEqual({r["tenant"] for r in records}, {f"tenant-{i:04d}" for i in range(5)})

    def test_encoded_input_size_is_exact_and_separate_from_envelope(self):
        for size in (128, 500, 32768):
            for record in generate(3, payload_bytes=size):
                self.assertEqual(len(encoded(record["input"])), size)
                self.assertEqual(record["input_bytes"], size)
                self.assertGreater(len(encoded(record)), size)

    def test_invalid_load_rejected_before_materialisation(self):
        for kwargs in ({"count": 0}, {"count": 10001}, {"count": True},
                       {"count": 1, "payload_bytes": 32769},
                       {"count": 10000, "payload_bytes": 32768},
                       {"count": 1, "distribution": "unknown"},
                       {"count": 1, "distribution": "skewed-80-percent", "tenants": 1}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                generate(**kwargs)


class OutcomeTests(unittest.TestCase):
    @staticmethod
    def snapshot(state, admitted=True):
        return {"steps": [{"id": "root-id", "parent": "", "key": "root",
                           "kind": "request", "state": state}],
                "messages": [{"kind": "Request", "task_id": "root-id"}] if admitted else []}

    def test_lost_response_and_known_failure_have_distinct_projections(self):
        unknown = classify(self.snapshot("blocked"), None, None)
        failed = classify(self.snapshot("failed"), None, 1)
        self.assertEqual(unknown["admission"], "admitted")
        self.assertEqual(unknown["execution"], "unknown")
        self.assertEqual(failed["execution"], "failed")
        self.assertIsNone(failed["started"])

    def test_prepared_snapshot_is_not_reported_as_definite_deferral(self):
        result = classify(self.snapshot("prepared", admitted=False), None, 1)
        self.assertEqual(result["admission"], "unknown")
        self.assertEqual(result["execution"], "unknown")

    def test_rejected_and_completed_have_different_admission_knowledge(self):
        rejected = classify(self.snapshot("rejected", admitted=False), None, 1)
        completed = classify(self.snapshot("completed"), None, 1)
        self.assertEqual(rejected, {"admission": "rejected", "execution": "not_started", "started": False})
        self.assertEqual(completed, {"admission": "admitted", "execution": "completed", "started": True})

    def test_process_failure_does_not_imply_failed_or_rejected_work(self):
        self.assertEqual(classify(None, None, 1),
                         {"admission": "unknown", "execution": "unknown", "started": None})
        self.assertEqual(classify(None, {"status": "completed"}, 0)["execution"], "completed")

    def test_request_identity_is_required_for_admission_evidence(self):
        snapshot = self.snapshot("prepared")
        snapshot["messages"][0]["task_id"] = "different-id"
        self.assertEqual(classify(snapshot, None, 1)["admission"], "unknown")

    def test_stable_empty_or_contradictory_observations_cannot_pass_completion(self):
        for snapshot in ({"steps": [], "messages": []}, self.snapshot("rejected", admitted=False),
                         self.snapshot("failed")):
            with self.subTest(snapshot=snapshot):
                self.assertEqual(classify(snapshot, {"status": "completed"}, 0)["execution"], "unknown")

    def test_zero_categories_remain_explicit(self):
        result = totals([])
        self.assertEqual(result["admission"], {"admitted": 0, "deferred": 0, "rejected": 0, "unknown": 0})
        self.assertEqual(set(result["execution"]), {"not_started", "started", "completed", "failed", "unknown"})

    def test_aggregate_counts_do_not_hide_per_request_duplicate_or_loss(self):
        one = {"request_id": "A", "admission": "admitted", "execution": "completed", "started": True}
        two = {**copy.deepcopy(one), "request_id": "B"}
        # Totals alone coincide. The runner retains the differing original IDs.
        self.assertEqual(totals([one, one]), totals([one, two]))
        self.assertNotEqual([r["request_id"] for r in [one, one]],
                            [r["request_id"] for r in [one, two]])

    def test_quantiles_are_nearest_rank_and_missing_is_explicit(self):
        self.assertEqual(latency_summary([1, 2, 3])["p95_ms"], 3)
        self.assertIsNone(latency_summary([])["p95_ms"])


if __name__ == "__main__":
    unittest.main()
