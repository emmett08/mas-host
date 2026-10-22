"""Check interchangeable fixture observations and exact-target failure cases."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("fixture_prepare", ROOT / "prepare.py")
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)


class Collectors(unittest.TestCase):
    def setUp(self):
        self.value = json.loads((ROOT / "input.json").read_bytes())
        self.fixture = json.loads((ROOT / "fixture.json").read_bytes())

    def run_collector(self, language, value=None, fixture=None, raw=None):
        with tempfile.TemporaryDirectory() as directory:
            fixture_file = Path(directory) / "fixture.json"
            fixture_file.write_text(json.dumps(self.fixture if fixture is None else fixture))
            if language == "python":
                argv = [sys.executable, "-I", "-B", str(ROOT / "python/collector.py")]
            else:
                self.assertTrue((ROOT / "typescript/dist/capability.mjs").is_file(), "Run npm ci --ignore-scripts && npm run build first")
                argv = ["node", str(ROOT / "typescript/cli.mjs"), "--module", str(ROOT / "typescript/dist/capability.mjs")]
            argv += ["--fixture", str(fixture_file), "--invocation", "fixture-invocation-1"]
            encoded = json.dumps(self.value if value is None else value).encode() if raw is None else raw
            return subprocess.run(argv, input=encoded, capture_output=True, timeout=5, check=False)

    def test_same_observation_and_original_application_host_identities(self):
        results = []
        for language in ("python", "typescript"):
            result = self.run_collector(language)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            value = json.loads(result.stdout)
            self.assertEqual(value["operationId"], self.value["operationId"])
            self.assertEqual(value["invocation_id"], "fixture-invocation-1")
            self.assertEqual(value["target"], self.value["target"])
            self.assertEqual(value["facts"]["ready_replicas"], 2)
            self.assertEqual(value["freshness"], "NOT_RUN")
            self.assertEqual(value["assessment"], "NOT_RUN")
            results.append(value)
        self.assertEqual(*results)

    def test_each_changed_target_field_is_rejected(self):
        for key in self.value["target"]:
            value = copy.deepcopy(self.value)
            value["target"][key] = "changed"
            for language in ("python", "typescript"):
                with self.subTest(key=key, language=language):
                    result = self.run_collector(language, value)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(result.stdout)

    def test_unknown_fields_and_file_or_endpoint_selection_are_rejected(self):
        for key in ("endpoint", "fixture", "token", "command"):
            value = {**self.value, key: "synthetic-private-marker"}
            for language in ("python", "typescript"):
                result = self.run_collector(language, value)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(result.stdout)
                self.assertNotIn(b"synthetic-private-marker", result.stderr)

    def test_quantities_cannot_be_booleans_fractional_negative_or_unbounded(self):
        for quantity in (True, 1.5, 2.0, -1, 101):
            fixture = copy.deepcopy(self.fixture)
            fixture["deployment"]["status"]["readyReplicas"] = quantity
            for language in ("python", "typescript"):
                self.assertNotEqual(self.run_collector(language, fixture=fixture).returncode, 0)

    def test_transport_rejects_duplicate_members_invalid_utf8_and_oversize_input(self):
        normal = json.dumps(self.value).encode()
        cases = [b'{"operationId":"first",' + normal[1:], b'\xff', b' ' * 8193]
        for raw in cases:
            for language in ("python", "typescript"):
                result = self.run_collector(language, raw=raw)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(result.stdout)

    def test_prepared_packages_have_same_contract_with_distinct_approved_implementation(self):
        with tempfile.TemporaryDirectory() as directory:
            packages = []
            for language in ("python", "typescript"):
                destination = PREPARE.prepare(language, Path(directory) / language)
                package = json.loads((destination / "capability.json").read_bytes())
                policy = json.loads((destination / "approvals.json").read_bytes())
                packages.append(package)
                self.assertEqual(policy["approved_packages"], [PREPARE.digest(PREPARE.canonical(package))])
                self.assertEqual(package["requires"]["credential_references"], [])
                self.assertEqual(package["recovery"]["replay"], "retained-observation")
                self.assertTrue(all((destination / item["path"]).is_file() for item in package["files"]))
            self.assertEqual(packages[0]["input_schema_sha256"], packages[1]["input_schema_sha256"])
            self.assertEqual(packages[0]["output_schema_sha256"], packages[1]["output_schema_sha256"])
            self.assertNotEqual(packages[0]["implementation_sha256"], packages[1]["implementation_sha256"])


if __name__ == "__main__":
    unittest.main()
