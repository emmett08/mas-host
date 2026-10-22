"""Validate case projections without granting authority to supplied JSON records."""
import copy
import unittest

from privacy_response import CaseEvent, Notification, ResponseError, case_status


PARENT = "task-" + "1" * 64
TARGET = "2" * 64
EVENT = CaseEvent("handling-withheld", "withheld", "blocked").safe_record()


def view(key, kind, output, execution, scope, effect):
    return {"version": "mas-governed-effect-view/1", "namespace_ref": "3" * 64,
            "actor": "responder", "parent_invocation": PARENT, "key": key,
            "effect_id": "eff-" + effect * 64, "state": "completed", "host_settlement": "completed",
            "external_execution": execution, "payload_disposition": "available",
            "descriptor": {"format": kind, "event": copy.deepcopy(EVENT), "target_binding": TARGET},
            "output": output, "evidence": {"version": "mas-governed-effect-evidence/1",
            "external_execution": execution, "payload_disposition": "available",
            "reported_evidence": {"execution": execution, "execution_scope": scope}}}


def records(submission="accepted", certainty="completed", retry_after=0):
    intent = view("privacy-case-intent", "mas-privacy-case-intent/1",
                  {"case_retained": True, "notification_pending": True}, "completed", "host-case-intent", "4")
    notification = view("privacy-notification", "mas-privacy-notification-submit/1",
                        {"submission": submission, "recipient_delivery": "unknown",
                         "retry_after_seconds": retry_after, "response_metadata": "withheld"},
                        certainty, "notification-endpoint-submission", "5")
    return intent, notification


class CaseStatusTests(unittest.TestCase):
    def test_completed_case_and_notification_preserve_exact_original_identities(self):
        intent, notification = records()
        before = copy.deepcopy((intent, notification))
        result = case_status(intent, notification)
        expected = Notification.for_invocation(PARENT, CaseEvent("handling-withheld", "withheld", "blocked"))
        self.assertEqual(result["version"], "mas-privacy-case-status/1")
        self.assertEqual(result["case_id"], expected.case_id)
        self.assertEqual(result["notification"]["action_id"], expected.action_id)
        self.assertEqual(result["notification"]["submission"], "accepted")
        self.assertEqual(result["notification"]["external_execution"], "completed")
        self.assertEqual(result["notification"]["recipient_delivery"], "unknown")
        self.assertEqual(result["notification"]["binding"], "matched")
        self.assertEqual((intent, notification), before)

    def test_refusal_throttle_and_unsent_remain_separate_from_acceptance(self):
        for submission, certainty, after in (("refused", "refused", 0), ("throttled", "refused", 60),
                                              ("not-sent", "not-sent", 0), ("refused", "not-sent", 0)):
            with self.subTest(submission=submission, certainty=certainty):
                result = case_status(*records(submission, certainty, after))["notification"]
                self.assertEqual(result["submission"], submission)
                self.assertEqual(result["external_execution"], certainty)
                self.assertEqual(result["retry_after_seconds"], after)
                self.assertEqual(result["recipient_delivery"], "unknown")

    def test_withheld_record_preserves_known_execution_without_inventing_submission(self):
        intent, notification = records()
        notification.update(payload_disposition="withheld", descriptor=None, output=None, evidence=None)
        result = case_status(intent, notification)["notification"]
        self.assertEqual(result["host_settlement"], "completed")
        self.assertEqual(result["external_execution"], "completed")
        self.assertEqual(result["payload_disposition"], "withheld")
        self.assertEqual(result["binding"], "unavailable")
        self.assertEqual(result["submission"], "unknown")
        self.assertIsNone(result["action_id"])
        self.assertIsNone(result["retry_after_seconds"])

    def test_unsettled_and_unrecorded_states_never_create_permission_to_send_again(self):
        for state in ("prepared", "executing", "unknown", "blocked", "failed", "rejected", "not-recorded"):
            intent, notification = records()
            settlement = "pending" if state in ("prepared", "executing") else "failed" if state in ("failed", "rejected") else "unknown"
            notification.update(state=state, host_settlement=settlement, external_execution="unknown",
                                output=None, evidence=None, descriptor=None, payload_disposition="unavailable")
            if state == "not-recorded":
                notification.update(effect_id=None, payload_disposition="unavailable", descriptor=None)
            with self.subTest(state=state):
                projected = case_status(intent, notification)["notification"]
                self.assertEqual(projected["record_state"], state)
                self.assertEqual(projected["external_execution"], "unknown")
                self.assertEqual(projected["submission"], "unknown")
                self.assertIsNone(projected["retry_after_seconds"])

    def test_a_generic_lookup_failure_is_not_an_absent_notification_record(self):
        intent, _ = records()
        for failed in (None, {}, {"error": {"code": -32001, "message": "effect_not_found"}}):
            with self.subTest(failed=failed), self.assertRaises(ResponseError):
                case_status(intent, failed)

    def test_scope_actor_parent_and_logical_keys_must_match(self):
        for field, value in (("namespace_ref", "6" * 64), ("actor", "other"),
                             ("parent_invocation", "task-" + "7" * 64), ("key", "different-key")):
            intent, notification = records()
            notification[field] = value
            with self.subTest(field=field), self.assertRaises(ResponseError):
                case_status(intent, notification)

    def test_changed_event_target_route_or_format_cannot_borrow_another_notification(self):
        for mutate in (
            lambda value: value["descriptor"].update(target_binding="6" * 64),
            lambda value: value["descriptor"]["event"].update(event_version=2),
            lambda value: value["descriptor"]["event"].update(route="security-response"),
            lambda value: value["descriptor"].update(format="another-operation/1"),
        ):
            intent, notification = records()
            mutate(notification)
            with self.assertRaises(ResponseError):
                case_status(intent, notification)

    def test_unavailable_or_incomplete_case_intent_does_not_create_a_case(self):
        for mutate in (
            lambda value: value.update(payload_disposition="withheld", descriptor=None, output=None, evidence=None),
            lambda value: value.update(state="unknown", host_settlement="unknown", external_execution="unknown", output=None, evidence=None),
            lambda value: value["output"].update(case_retained=False),
            lambda value: value["output"].update(notification_pending=1),
        ):
            intent, notification = records()
            mutate(intent)
            with self.assertRaises(ResponseError):
                case_status(intent, notification)

    def test_closed_versioned_records_reject_private_fields_and_malformed_identifiers(self):
        for mutate in (
            lambda value: value.update(private_note="person@example.invalid"),
            lambda value: value.update(version="mas-governed-effect-view/2"),
            lambda value: value.update(effect_id="supplied-email@example.invalid"),
            lambda value: value.update(parent_invocation="task-" + "g" * 64),
            lambda value: value["descriptor"].update(private_note="person@example.invalid"),
            lambda value: value["output"].update(private_note="person@example.invalid"),
            lambda value: value["descriptor"]["event"].update(categories=["person@example.invalid"], finding_count=1),
            lambda value: value.update(state=["unknown"]),
        ):
            intent, notification = records()
            mutate(notification)
            with self.assertRaises(ResponseError) as failure:
                case_status(intent, notification)
            self.assertNotIn("person@example.invalid", str(failure.exception))

    def test_receipt_certainty_and_payload_consistency_are_checked(self):
        for mutate in (
            lambda value: value.update(host_settlement="unknown"),
            lambda value: value.update(external_execution="refused"),
            lambda value: value["evidence"].update(external_execution="refused"),
            lambda value: value["evidence"]["reported_evidence"].update(execution_scope="other-operation"),
            lambda value: value["output"].update(recipient_delivery="delivered"),
            lambda value: value["output"].update(retry_after_seconds=True),
            lambda value: value["output"].update(retry_after_seconds=60),
        ):
            intent, notification = records()
            mutate(notification)
            with self.assertRaises(ResponseError):
                case_status(intent, notification)


if __name__ == "__main__":
    unittest.main()
