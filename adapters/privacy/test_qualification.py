"""Adversarial qualification checks, including an optional actual-host CLI path."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("qualification", ROOT / "qualification.py")
q = importlib.util.module_from_spec(spec)
spec.loader.exec_module(q)


def encoded(value):
    return json.dumps(value, separators=(",", ":")).encode()


def source(identifier="a", text="é🙂 x", positive=True, unit="unit-a"):
    return {"version": "mas-privacy-labelled-source/1", "id": identifier, "unit_id": unit,
        "stratum": "synthetic-en", "text": text,
        "prohibited": [{"start": 2, "end": 6, "category": "person", "entity_id": "entity-1"}] if positive else [],
        "required": [{"start": len(text.encode()) - 1, "end": len(text.encode())}]}


def manifest(items):
    raw = b"".join(encoded(item) + b"\n" for item in items)
    value = {"version": "mas-privacy-qualification-manifest/1", "corpus_sha256": q.digest(raw),
        "detector": {"bundle_sha256": "0" * 64, "code_revision": "a" * 40,
            "model_revision": "b" * 40, "backend": "opf-cpu-bfloat16/1", "adapter": "mas-privacy-opf/1"},
        "taxonomy": "mas-privacy-taxonomy/2", "offset_unit": "utf8-bytes", "confidence": 0.95,
        "evidence_scope": "synthetic", "annotation_evidence": "synthetic-test-fixture",
        "sampling_evidence": "synthetic-test-fixture", "independent_units_attested": False,
        "strata": {"synthetic-en": {"min_units": 1, "min_negative_units": 1,
            "max_residual_upper": 0.99, "max_false_block_upper": 0.99,
            "min_complete_fraction": 1, "min_required_preserved_fraction": 1}}}
    return value, raw


def report(item, config, predicted=None, completion="complete"):
    predicted = item["prohibited"] if predicted is None else predicted
    findings = [{"start": a["start"], "end": a["end"], "categories": [a["category"]],
        "recognisers": ["synthetic/1"]} for a in predicted]
    nbytes = len(item["text"].encode())
    return {"version": "mas-privacy-inspection/1", "status": "assessed", "source_bytes": nbytes,
        "detector": config["detector"], "completion": completion, "finding_count": len(findings),
        "restricted_assessment": {"version": "mas-combined-privacy-assessment/1",
            "taxonomy": config["taxonomy"], "completion": completion, "findings": findings,
            "neural": {"version": "mas-neural-assessment/1", "input_bytes": nbytes,
                "detector": config["detector"], "completion": completion, "offset_unit": "utf8-bytes",
                "extraction": "utf8-text/1", "normalisation": "none/1",
                "checked_ranges": [{"start": 0, "end": nbytes}] if nbytes else [],
                "omissions": [] if completion == "complete" else ["model-unavailable"]}}}


def envelopes(items, config, manifest_sha):
    return [{"version": "mas-privacy-qualified-assessment/1", "id": item["id"],
        "source_sha256": q.digest(item["text"].encode()), "manifest_sha256": manifest_sha,
        "latency_ms": 2.5, "report": report(item, config)} for item in items]


class ExactBounds(unittest.TestCase):
    def test_zero_failures_planning_bound_and_endpoints(self):
        self.assertAlmostEqual(q.binomial_upper(0, 299), 1 - 0.05 ** (1 / 299), places=14)
        self.assertLess(q.binomial_upper(0, 299), 0.01)
        self.assertGreater(q.binomial_upper(0, 298), 0.01)
        self.assertIsNone(q.binomial_upper(0, 0))
        self.assertEqual(q.binomial_upper(9, 9), 1)
        # For n=2,k=1, P(X<=1)=1-p^2=0.05 at this exact bound.
        self.assertAlmostEqual(q.binomial_upper(1, 2), 0.95 ** 0.5, places=12)

    def test_bound_is_monotone_and_conservative(self):
        values = [q.binomial_upper(k, 40) for k in range(41)]
        self.assertEqual(values, sorted(values))
        for k, bound in enumerate(values):
            self.assertGreaterEqual(bound, k / 40)


class CorpusAndAssessment(unittest.TestCase):
    def setUp(self):
        self.items = [source(), source("b", positive=False, unit="unit-b")]
        self.config, self.raw = manifest(self.items)
        self.manifest_raw = encoded(self.config)
        self.sha = q.digest(self.manifest_raw)
        self.records = envelopes(self.items, self.config, self.sha)

    def test_exact_source_manifest_and_unicode_offsets(self):
        self.assertEqual(q.manifest(self.manifest_raw), self.config)
        self.assertEqual(q.corpus(self.raw, self.config), self.items)
        value = q.evaluate(self.items, self.records, self.config, self.sha)
        self.assertEqual(value["status"], "SYNTHETIC_CONFORMANCE_ONLY")
        category = value["strata"]["synthetic-en"]["categories"]["person"]
        self.assertEqual(category["utf8_bytes"]["true_positive"], 4)
        self.assertEqual(category["unicode_codepoints"]["true_positive"], 1)
        self.assertEqual(category["exact_entities"]["recall"], 1)
        self.assertFalse(value["release_authority_granted"])

    def test_changed_source_same_byte_length_or_frozen_manifest_is_rejected(self):
        changed = copy.deepcopy(self.items)
        changed[0]["text"] = "é🙂 z"
        with self.assertRaises(q.QualificationError):
            q.evaluate(changed, self.records, self.config, self.sha)
        with self.assertRaises(q.QualificationError):
            q.evaluate(self.items, self.records, self.config, "1" * 64)
        with self.assertRaises(q.QualificationError):
            q.corpus(self.raw + b"\n", self.config)

    def test_silent_truncation_wrong_units_changed_detector_or_range_reject(self):
        for mutate in (
            lambda v: v["restricted_assessment"]["neural"].update(checked_ranges=[{"start": 0, "end": 2}]),
            lambda v: v["restricted_assessment"]["neural"].update(offset_unit="unicode-codepoints"),
            lambda v: v.update(detector={}),
            lambda v: v["restricted_assessment"]["findings"][0].update(start=3),
            lambda v: v["restricted_assessment"]["findings"][0].update(categories=["unknown"]),
        ):
            faulty = copy.deepcopy(self.records[0]["report"])
            mutate(faulty)
            with self.assertRaises(q.QualificationError):
                q.validate_report(faulty, self.items[0], self.config)

    def test_replay_order_and_duplicate_ids_reject(self):
        with self.assertRaises(q.QualificationError):
            q.evaluate(self.items, self.records[::-1], self.config, self.sha)
        duplicate = self.items + [copy.deepcopy(self.items[0])]
        config, raw = manifest(duplicate)
        with self.assertRaises(q.QualificationError):
            q.corpus(raw, config)

    def test_missing_domain_annotation_or_sampling_attestation_reject(self):
        for field, value in (("annotation_evidence", ""), ("sampling_evidence", ""),
                             ("independent_units_attested", False)):
            config = copy.deepcopy(self.config)
            config.update(evidence_scope="deployment-domain", independent_units_attested=True)
            config[field] = value
            with self.assertRaises(q.QualificationError):
                q.manifest(encoded(config))

    def test_incomplete_scan_is_withheld_but_fails_complete_gate_and_false_blocks(self):
        self.records[1]["report"] = report(self.items[1], self.config, completion="failed")
        value = q.evaluate(self.items, self.records, self.config, self.sha)
        self.assertEqual(value["status"], "THRESHOLDS_NOT_MET")
        stratum = value["strata"]["synthetic-en"]
        self.assertEqual(stratum["counts"]["residual_documents"], 0)
        self.assertEqual(stratum["counts"]["false_block_documents"], 1)
        self.assertEqual(stratum["completion"], {"complete": 1, "failed": 1})

    def test_residual_disclosure_and_required_fact_loss_are_independent(self):
        missed = copy.deepcopy(self.records)
        missed[0]["report"] = report(self.items[0], self.config, predicted=[])
        value = q.evaluate(self.items, missed, self.config, self.sha)["strata"]["synthetic-en"]
        self.assertEqual(value["counts"]["residual_documents"], 1)
        self.assertEqual(value["required_preserved_fraction"], 1)
        excess = copy.deepcopy(self.records)
        predicted = self.items[0]["prohibited"] + [{"start": 7, "end": 8, "category": "person"}]
        excess[0]["report"] = report(self.items[0], self.config, predicted=predicted)
        value = q.evaluate(self.items, excess, self.config, self.sha)["strata"]["synthetic-en"]
        self.assertEqual(value["counts"]["residual_documents"], 0)
        self.assertEqual(value["required_preserved_fraction"], 0.5)

    def test_conversation_repetition_is_one_inference_unit(self):
        items = [source("a", unit="conversation-1"), source("c", unit="conversation-1"),
            source("b", positive=False, unit="conversation-2")]
        config, _raw = manifest(items)
        sha = q.digest(encoded(config))
        records = envelopes(items, config, sha)
        records[1]["report"] = report(items[1], config, predicted=[])
        value = q.evaluate(items, records, config, sha)["strata"]["synthetic-en"]
        self.assertEqual(value["independent_units"], 2)
        self.assertEqual(value["repeated_entities"], {"groups": 1, "fully_detected": 0})
        self.assertEqual(value["counts"]["residual_documents"], 1)

    def test_empty_or_unmeasured_stratum_cannot_pass(self):
        self.config["strata"]["unmeasured-en"] = copy.deepcopy(self.config["strata"]["synthetic-en"])
        value = q.evaluate(self.items, self.records, self.config, self.sha)
        self.assertFalse(value["thresholds_met"])
        self.assertIsNone(value["strata"]["unmeasured-en"]["residual_unit_upper"])

    def test_many_correlated_documents_do_not_establish_299_trials(self):
        items = [source("doc-" + str(i), unit="one-template") for i in range(299)]
        items.append(source("negative", positive=False, unit="second-template"))
        config, _raw = manifest(items)
        config["strata"]["synthetic-en"]["min_units"] = 299
        sha = q.digest(encoded(config))
        value = q.evaluate(items, envelopes(items, config, sha), config, sha)
        stratum = value["strata"]["synthetic-en"]
        self.assertEqual(stratum["documents"], 300)
        self.assertEqual(stratum["independent_units"], 2)
        self.assertFalse(stratum["checks"]["minimum_units"])
        self.assertGreater(stratum["residual_unit_upper"], 0.5)

    def test_wrong_category_can_remove_bytes_while_lowering_category_recall(self):
        wrong = copy.deepcopy(self.records)
        wrong[0]["report"] = report(self.items[0], self.config, predicted=[
            {"start": 2, "end": 6, "category": "credential"}])
        value = q.evaluate(self.items, wrong, self.config, self.sha)["strata"]["synthetic-en"]
        self.assertEqual(value["counts"]["residual_documents"], 0)
        self.assertEqual(value["categories"]["person"]["unicode_codepoints"]["recall"], 0)
        self.assertEqual(value["categories"]["credential"]["unicode_codepoints"]["precision"], 0)

    def test_contradictory_annotations_and_wrong_unicode_units_reject(self):
        for mutate in (lambda v: v[0]["required"].append({"start": 2, "end": 6}),
                       lambda v: v[0]["prohibited"][0].update(start=3)):
            items = copy.deepcopy(self.items)
            mutate(items)
            config, raw = manifest(items)
            with self.assertRaises(q.QualificationError):
                q.corpus(raw, config)

    def test_duplicate_keys_and_nonfinite_json_reject(self):
        for raw in (b'{"a":1,"a":2}', b'{"a":NaN}'):
            with self.assertRaises(q.QualificationError):
                q.parse(raw)

    def test_cli_replay_emits_only_aggregate_restricted_result(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "manifest.json").write_bytes(self.manifest_raw)
            (root / "corpus.jsonl").write_bytes(self.raw)
            (root / "replay.jsonl").write_bytes(b"".join(encoded(row) + b"\n" for row in self.records))
            command = [sys.executable, str(ROOT / "qualification.py"), "--manifest", str(root / "manifest.json"),
                "--corpus", str(root / "corpus.jsonl"), "--replay", str(root / "replay.jsonl"),
                "--output", str(root / "result.json")]
            result = subprocess.run(command, capture_output=True, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("é🙂 x", (root / "result.json").read_text())
            self.assertEqual((root / "result.json").stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(result.stdout)["status"], "SYNTHETIC_CONFORMANCE_ONLY")


class ActualHost(unittest.TestCase):
    def test_real_privacy_inspect_collection_and_exact_replay(self):
        host = Path(os.environ.get("MAS_PRIVACY_QUALIFICATION_HOST", ROOT.parent.parent / "target/debug/mas-host"))
        if not host.is_file():
            self.skipTest("Build mas-host or set MAS_PRIVACY_QUALIFICATION_HOST for actual-host conformance")
        items = [source("positive", "Contact alice@example.invalid count=7", False),
                 source("negative", "Public count=7", False, "unit-b")]
        start = len("Contact ".encode())
        items[0]["prohibited"] = [{"start": start, "end": start + len("alice@example.invalid"),
            "category": "email-address", "entity_id": "person-1"}]
        config, corpus_raw = manifest(items)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            worker = ROOT / "qualification/fixture_worker.py"
            bundle = {"version": "mas-privacy-bundle/1", "code_revision": "a" * 40,
                "model_revision": "b" * 40, "backend": "opf-cpu-bfloat16/1", "adapter": "mas-privacy-opf/1",
                "worker_sha256": q.digest(worker.read_bytes()), "adapter_files": {}, "files": {}, "runtime": {},
                "settings": {"max_input_bytes": 16384, "max_response_bytes": 65536, "max_findings": 128}}
            bundle_raw = encoded(bundle)
            config["detector"]["bundle_sha256"] = q.digest(bundle_raw)
            (root / "bundle.json").write_bytes(bundle_raw)
            (root / "manifest.json").write_bytes(encoded(config))
            (root / "corpus.jsonl").write_bytes(corpus_raw)
            common = [sys.executable, str(ROOT / "qualification.py"), "--manifest", str(root / "manifest.json"),
                "--corpus", str(root / "corpus.jsonl")]
            collect = subprocess.run(common + ["--host", str(host), "--python", "/usr/bin/python3",
                "--bundle-manifest", str(root / "bundle.json"), "--assets", str(root), "--worker", str(worker),
                "--timeout-ms", "10000", "--assessments-output", str(root / "assessments.jsonl"),
                "--output", str(root / "fresh.json")], capture_output=True, check=False, timeout=40)
            self.assertEqual(collect.returncode, 0, collect.stderr)
            replay = subprocess.run(common + ["--replay", str(root / "assessments.jsonl"), "--output", str(root / "replay.json")],
                capture_output=True, check=False)
            self.assertEqual(replay.returncode, 0, replay.stderr)
            self.assertEqual(json.loads((root / "fresh.json").read_text()), json.loads((root / "replay.json").read_text()))
            self.assertEqual((root / "assessments.jsonl").stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
