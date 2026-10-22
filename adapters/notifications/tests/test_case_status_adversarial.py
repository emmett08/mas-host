"""Challenge cross-layer case projections using independently constructed host views."""
import copy
import unittest

from privacy_response import CaseEvent, ResponseError, case_status


def views():
    event = CaseEvent("handling-withheld", "withheld", "blocked").safe_record()
    common = {"version": "mas-governed-effect-view/1", "namespace_ref": "a" * 64,
              "actor": "responder", "parent_invocation": "task-" + "b" * 64,
              "state": "completed", "host_settlement": "completed",
              "external_execution": "completed", "payload_disposition": "available"}
    intent = {**common, "key": "privacy-case-intent", "effect_id": "eff-" + "c" * 64,
              "descriptor": {"format": "mas-privacy-case-intent/1", "event": event,
                             "target_binding": "d" * 64},
              "output": {"case_retained": True, "notification_pending": True},
              "evidence": {"version": "mas-governed-effect-evidence/1",
                           "external_execution": "completed", "payload_disposition": "available",
                           "reported_evidence": {"execution": "completed", "execution_scope": "host-case-intent"}}}
    notification = {**common, "key": "privacy-notification", "effect_id": "eff-" + "e" * 64,
                    "descriptor": {"format": "mas-privacy-notification-submit/1", "event": copy.deepcopy(event),
                                   "target_binding": "d" * 64},
                    "output": {"submission": "accepted", "recipient_delivery": "unknown",
                               "retry_after_seconds": 0, "response_metadata": "withheld"},
                    "evidence": {"version": "mas-governed-effect-evidence/1",
                                 "external_execution": "completed", "payload_disposition": "available",
                                 "reported_evidence": {"execution": "completed", "execution_scope": "notification-endpoint-submission"}}}
    return intent, notification


def inspection():
    return {"version": "mas-governed-inspection-receipt/1",
            "contract": {"version": "mas-governed-inspection/1",
                         "detector": {"bundle_sha256": "1" * 64, "code_revision": "2" * 40,
                                      "model_revision": "3" * 40, "backend": "opf-cpu-bfloat16/1", "adapter": "mas-privacy-opf/1"},
                         "configuration_binding": "4" * 64, "timeout_ms": 10000,
                         "destination": "local-detector", "recipient": "privacy-inspector",
                         "extraction": "json-text-components/1", "max_boundary_bytes": 16384,
                         "max_text_units": 128, "release_mode": "structured-projection-only"},
            "original_binding": "5" * 64, "projection_binding": "6" * 64,
            "completion": "complete", "coverage": "complete-within-json-text-components/1",
            "original_findings_count": 1, "projected_findings_count": 0,
            "text_units": 12, "inspected_sources": 6, "boundary_bytes": 1024}


class CaseProjectionBoundaryTests(unittest.TestCase):
    def test_real_n2_completion_evidence_can_include_a_valid_inspection_receipt(self):
        intent, notification = views()
        baseline = case_status(intent, notification)
        intent["evidence"]["inspection"] = inspection()
        notification["evidence"]["inspection"] = inspection()
        before = copy.deepcopy((intent, notification))
        self.assertEqual(case_status(intent, notification), baseline)
        self.assertEqual((intent, notification), before)
        self.assertNotIn("inspection", baseline)

    def test_optional_inspection_is_validated_before_its_metadata_is_discarded(self):
        for bad in (None, [], "private-person@example.invalid",
                    {**inspection(), "extra": "private-person@example.invalid"},
                    {**inspection(), "projected_findings_count": 1},
                    {**inspection(), "original_findings_count": True},
                    {**inspection(), "inspected_sources": 13},
                    {**inspection(), "boundary_bytes": 16385},
                    {**inspection(), "coverage": "partial"}):
            intent, notification = views()
            notification["evidence"]["inspection"] = bad
            with self.subTest(receipt=bad), self.assertRaises(ResponseError) as error:
                case_status(intent, notification)
            self.assertNotIn("private-person", str(error.exception))

    def test_impossible_host_state_settlement_pairs_do_not_become_case_facts(self):
        mapping = {"not-recorded": "unknown", "prepared": "pending", "executing": "pending",
                   "unknown": "unknown", "blocked": "unknown", "failed": "failed", "rejected": "failed"}
        for state, expected in mapping.items():
            for settlement in ("pending", "failed", "unknown"):
                if settlement == expected:
                    continue
                intent, notification = views()
                notification.update(state=state, host_settlement=settlement,
                                    external_execution="unknown", payload_disposition="unavailable",
                                    descriptor=None, output=None, evidence=None)
                if state == "not-recorded":
                    notification["effect_id"] = None
                with self.subTest(state=state, settlement=settlement), self.assertRaises(ResponseError):
                    case_status(intent, notification)

    def test_missing_or_unsettled_effects_preserve_uncertainty_without_a_retry_projection(self):
        for state, settlement in (("not-recorded", "unknown"), ("executing", "pending"),
                                  ("unknown", "unknown"), ("rejected", "failed")):
            intent, notification = views()
            notification.update(state=state, host_settlement=settlement,
                                external_execution="unknown", payload_disposition="unavailable",
                                descriptor=None, output=None, evidence=None)
            if state == "not-recorded":
                notification["effect_id"] = None
            projected = case_status(intent, notification)["notification"]
            self.assertEqual(projected["record_state"], state)
            self.assertEqual(projected["host_settlement"], settlement)
            self.assertEqual(projected["external_execution"], "unknown")
            self.assertEqual(projected["submission"], "unknown")
            self.assertIsNone(projected["action_id"])
            self.assertIsNone(projected["retry_after_seconds"])

    def test_different_logical_effect_keys_cannot_borrow_the_same_retained_effect_identity(self):
        intent, notification = views()
        notification["effect_id"] = intent["effect_id"]
        with self.assertRaises(ResponseError):
            case_status(intent, notification)

    def test_completed_metadata_without_a_descriptor_never_claims_endpoint_acceptance(self):
        intent, notification = views()
        notification.update(descriptor=None, output=None, evidence=None, payload_disposition="withheld")
        projected = case_status(intent, notification)["notification"]
        self.assertEqual(projected["host_settlement"], "completed")
        self.assertEqual(projected["external_execution"], "completed")
        self.assertEqual(projected["submission"], "unknown")
        self.assertEqual(projected["binding"], "unavailable")
        self.assertIsNone(projected["action_id"])


if __name__ == "__main__":
    unittest.main()
