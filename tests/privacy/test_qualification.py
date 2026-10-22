"""Negative controls ensure synthetic qualification rejects false passes."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import qualify


class QualificationTests(unittest.TestCase):
    def setUp(self):
        self.detector = {"bundle_sha256": "a" * 64, "code_revision": "b" * 40,
                         "model_revision": "c" * 40, "backend": "opf-cpu-bfloat16/1",
                         "adapter": "mas-privacy-opf/1"}
        self.case = {"id": "synthetic", "text": "🙂 Alice alice@example.com",
                     "minimum_findings": 2, "expect_no_findings": False,
                     "expected_entities": [
                         {"label": "private_person", "start": 5, "end": 10},
                         {"label": "private_email", "start": 11, "end": 28}]}
        self.value = {"version": "mas-privacy-inspection/1", "status": "assessed",
                      "source_bytes": 28, "completion": "complete", "finding_count": 2,
                      "release_permitted": False, "quality_qualified": False,
                      "disposition": "withhold", "detector": self.detector,
                      "restricted_assessment": {
                          "completion": "complete", "release_qualified": False,
                          "findings": [{}, {}], "neural": {
                              "input_bytes": 28, "detector": self.detector,
                              "completion": "complete", "omissions": [],
                              "offset_unit": "utf8-bytes", "score_semantics": "not-exposed",
                              "checked_ranges": [{"start": 0, "end": 28}],
                              "findings": copy.deepcopy(self.case["expected_entities"])}}}

    def validate(self, value=None, errors=b""):
        return qualify.validate_report(json.dumps(self.value if value is None else value).encode(),
                                       errors, self.detector, self.case)

    def test_complete_byte_correct_synthetic_result_passes(self):
        result = self.validate()
        self.assertEqual(result["neural_finding_count"], 2)
        self.assertTrue(all(item["covered"] for item in result["expected_entity_checks"]))

    def test_coverage_check_permits_surrounding_whitespace(self):
        self.value["restricted_assessment"]["neural"]["findings"][0]["start"] = 4
        self.validate()

    def test_codepoint_offset_mistaken_for_byte_offset_fails(self):
        self.value["restricted_assessment"]["neural"]["findings"][0]["start"] = 1
        with self.assertRaisesRegex(qualify.QualificationFailure, "split-utf8-codepoint"):
            self.validate()

    def test_missing_entity_cannot_pass_with_equal_aggregate_count(self):
        findings = self.value["restricted_assessment"]["neural"]["findings"]
        findings[1] = copy.deepcopy(findings[0])
        with self.assertRaisesRegex(qualify.QualificationFailure, "expected-entity-missed"):
            self.validate()

    def test_source_leak_is_found_after_json_unescaping(self):
        self.value["diagnostic"] = self.case["text"]
        with self.assertRaisesRegex(qualify.QualificationFailure, "source-leaked"):
            self.validate()

    def test_stderr_is_rejected(self):
        with self.assertRaisesRegex(qualify.QualificationFailure, "unexpected-stderr"):
            self.validate(errors=b"synthetic diagnostic")

    def test_policy_release_and_quality_claims_are_rejected(self):
        for field in ("release_permitted", "quality_qualified"):
            value = copy.deepcopy(self.value)
            value[field] = True
            with self.assertRaisesRegex(qualify.QualificationFailure, "release-authority"):
                self.validate(value)

    def test_failed_scan_or_missing_coverage_cannot_pass(self):
        failed = copy.deepcopy(self.value)
        failed["completion"] = "failed"
        with self.assertRaisesRegex(qualify.QualificationFailure, "incomplete-assessment"):
            self.validate(failed)
        self.value["restricted_assessment"]["neural"]["checked_ranges"] = []
        with self.assertRaisesRegex(qualify.QualificationFailure, "coverage-gap"):
            self.validate()

    def test_different_detector_bundle_is_rejected(self):
        value = copy.deepcopy(self.value)
        value["detector"]["bundle_sha256"] = "d" * 64
        with self.assertRaisesRegex(qualify.QualificationFailure, "detector-binding"):
            self.validate(value)

    def test_duplicate_keys_and_nonfinite_json_are_rejected(self):
        for raw in (b'{"completion":"failed","completion":"complete"}', b'{"n":NaN}'):
            with self.assertRaises(qualify.QualificationFailure):
                qualify.parse(raw)

    def test_outer_cli_deadline_is_bounded_without_model_assets(self):
        start = time.monotonic()
        with self.assertRaisesRegex(qualify.QualificationFailure, "cli-deadline"):
            qualify.run_case([sys.executable, "-c", "import time; time.sleep(10)"], "", 0.05)
        self.assertLess(time.monotonic() - start, 3)

    def test_partial_progress_remains_running_until_every_case_passes(self):
        expected = [f"case-{number}" for number in range(6)]
        completed = [{"id": name, "status": "PASS"} for name in expected]
        for count in range(6):
            self.assertEqual(qualify.receipt_state(expected, completed[:count]), "RUNNING")
        self.assertEqual(qualify.receipt_state(expected, completed), "PASS")
        self.assertEqual(qualify.receipt_state([], []), "RUNNING")
        completed[-1]["status"] = "FAIL"
        self.assertEqual(qualify.receipt_state(expected, completed), "FAIL")

    def test_preflight_failure_replaces_a_stale_pass_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            receipt = Path(directory) / "receipt.json"
            receipt.write_text('{"status":"PASS"}')
            arguments = ["qualify.py", "--host", sys.executable, "--python", sys.executable,
                         "--manifest", str(Path(directory) / "missing.json"),
                         "--assets", directory, "--receipt", str(receipt)]
            with patch.object(sys, "argv", arguments):
                self.assertEqual(qualify.main(), 1)
            result = json.loads(receipt.read_text())
            self.assertEqual(result["status"], "FAIL")
            self.assertEqual(result["completed_case_count"], 0)


if __name__ == "__main__":
    unittest.main()
