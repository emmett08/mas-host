"""Fault controls for the offline adapter without model dependencies or assets."""
import copy
import hashlib
import json
import io
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

from mas_privacy_opf import bundle, service
from mas_privacy_opf.contracts import (ADAPTER, BACKEND, CODE_REVISION, MODEL_REVISION, AdapterError,
    parse, request, response)
from mas_privacy_opf.inference import CheckedDecoder, CheckedModel, validate_findings
from mas_privacy_opf.transport import read_frame, write_frame
import provision


def fixture(text="Aé🙂 Z"):
    return {"version": "mas-privacy-worker-request/1", "source_id": "source-1", "source_token": "1" * 64,
            "input_bytes": len(text.encode()), "text": text,
            "detector": {"bundle_sha256": "0" * 64, "code_revision": CODE_REVISION,
                         "model_revision": MODEL_REVISION, "backend": BACKEND, "adapter": ADAPTER}}


class Tensor:
    def __init__(self, shape, values=None, finite=True):
        self.shape, self.values, self.finite = shape, values, finite
    def tolist(self):
        return self.values


class Contracts(unittest.TestCase):
    def test_strict_source_identity_byte_bounds_and_versions(self):
        value = fixture()
        self.assertEqual(request(value), value)
        for key, bad in [("input_bytes", len(value["text"])), ("input_bytes", True),
                         ("source_token", "not-a-token"), ("version", "future"),
                         ("text", "x" * 16385)]:
            wrong = copy.deepcopy(value); wrong[key] = bad
            with self.assertRaises(AdapterError): request(wrong)
        wrong = copy.deepcopy(value); wrong["detector"]["backend"] = "unknown"
        with self.assertRaises(AdapterError): request(wrong)
        wrong = copy.deepcopy(value); wrong["extra"] = "private-value"
        with self.assertRaises(AdapterError) as error: request(wrong)
        self.assertNotIn("private-value", str(error.exception))

    def test_duplicate_json_nonfinite_truncation_and_surrogates_reject(self):
        for raw in [b'{"x":1,"x":2}', b'{"x":NaN}', b'{"x":', b" " * 65537]:
            with self.assertRaises(AdapterError): parse(raw)
        value = fixture(); value["text"] = "\ud800"
        with self.assertRaises(AdapterError): request(value)

    def test_response_has_only_permitted_correlated_metadata(self):
        value = fixture("Private synthetic marker")
        result = response(value, "complete", [{"start": 0, "end": 7, "label": "private_person"}])
        self.assertEqual(result["source_token"], value["source_token"])
        self.assertEqual(result["checked_ranges"], [{"start": 0, "end": len(value["text"])}])
        self.assertNotIn(value["text"], json.dumps(result))
        for finding in [{"start": 0, "end": 1000, "label": "private_person"},
                        {"start": 0, "end": 2, "label": "private_person", "text": "Private"}]:
            with self.assertRaises(AdapterError): response(value, "complete", [finding])
        with self.assertRaises(AdapterError): response(value, "failed")
        with self.assertRaises(AdapterError): response(value, "partial", omissions=["private arbitrary error"])

    def test_character_coordinates_preserve_unicode_and_overlap_categories(self):
        value = fixture("é🙂N")
        state, findings, omissions = validate_findings([(1, 3, "private_person"), (1, 2, "private_email")], value["text"])
        self.assertEqual(state, "complete")
        self.assertEqual(findings[0], {"start": 1, "end": 2, "label": "private_email"})
        self.assertEqual(len(value["text"][:findings[0]["start"]].encode()), 2)
        self.assertEqual(len(value["text"][:findings[0]["end"]].encode()), 6)
        self.assertEqual(omissions, [])

    def test_positive_findings_survive_explicit_limit_as_partial(self):
        rows = [(i, i + 1, "private_person") for i in range(129)]
        state, findings, omissions = validate_findings(rows, "x" * 129)
        self.assertEqual((state, len(findings), omissions), ("partial", 128, ["detector-limit"]))
        with self.assertRaises(AdapterError): validate_findings([(0, 1, "unknown")], "a")


class CoverageGuards(unittest.TestCase):
    def test_decoder_length_failure_cannot_fall_back_to_argmax(self):
        class Decoder:
            def __init__(self, labels): self.labels = labels
            def decode(self, _scores): return self.labels
        for labels in (None, [], [0], [0, 33], [False, 0]):
            checked = CheckedDecoder(Decoder(labels), 2, lambda score: score.finite)
            with self.assertRaisesRegex(AdapterError, "decoder-incomplete"):
                checked.decode(Tensor((2, 33)))

    def test_missing_final_token_or_nonfinite_scores_cannot_be_complete(self):
        class Decoder:
            def decode(self, _scores): return [0, 0]
        for tensor in [Tensor((0, 33)), Tensor((1, 33)), Tensor((2, 32)), Tensor((2, 33), finite=False)]:
            checked = CheckedDecoder(Decoder(), 2, lambda score: score.finite)
            with self.assertRaises(AdapterError): checked.decode(tensor)

    def test_model_checks_original_tokens_full_mask_and_exact_output(self):
        model = lambda *args, **kwargs: Tensor((1, 2, 33))
        checked = CheckedModel(model, [5, 8], lambda score: score.finite)
        checked(Tensor((1, 2), [[5, 8]]), attention_mask=Tensor((1, 2), [[True, True]]))
        with self.assertRaises(AdapterError):
            checked(Tensor((1, 2), [[5, 8]]), attention_mask=Tensor((1, 2), [[True, True]]))
        for tokens, mask in [([[5]], [[True]]), ([[5, 9]], [[True, True]]), ([[5, 8]], [[True, False]])]:
            with self.assertRaises(AdapterError):
                CheckedModel(model, [5, 8], lambda score: score.finite)(Tensor((1, 2), tokens), attention_mask=Tensor((1, 2), mask))


class Assets(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        names = ["source/pyproject.toml", "source/opf/__init__.py", "source/opf/runtime.py",
                 "checkpoint/config.json", "checkpoint/dtypes.json", "checkpoint/model.safetensors",
                 "checkpoint/viterbi_calibration.json", "tokenizer-cache/" + bundle.TOKENIZER_KEY]
        self.files = {}
        for name in names:
            path = self.root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b"fixture")
            self.files[name] = {"bytes": 7, "sha256": hashlib.sha256(b"fixture").hexdigest()}
        self.manifest = {"files": self.files}

    def verify(self):
        with patch.object(bundle, "reviewed_files", return_value=self.files):
            bundle.verify_assets(self.manifest, self.root, self.root)

    def test_offline_assets_missing_changed_and_path_escape_fail(self):
        self.verify()
        (self.root / "checkpoint/config.json").write_bytes(b"changed")
        with self.assertRaises(AdapterError): self.verify()
        with self.assertRaises(AdapterError): bundle.relative_path(self.root, "../outside")

    def test_extra_importable_extension_bytecode_and_symlink_fail(self):
        for name in ["runtime.cpython-312-x86_64-linux-gnu.so", "unreviewed.pth", "runtime.pyc"]:
            path = self.root / "source/opf" / name; path.write_bytes(b"unreviewed")
            with self.assertRaises(AdapterError): self.verify()
            path.unlink()
        path = self.root / "source/opf/alias.py"; path.symlink_to("runtime.py")
        with self.assertRaises(AdapterError): self.verify()

    def test_runtime_unknown_or_missing_version_never_imports_model(self):
        manifest = {"runtime": {"python": "0.0.0", "packages": {}}}
        with self.assertRaises(AdapterError): bundle.verify_runtime(manifest, self.root)

    def test_provisioning_preserves_another_transfers_partial_file(self):
        path = self.root / "download.bin"
        partial = path.with_name(path.name + ".partial")
        partial.write_bytes(b"another transfer")
        record = {"bytes": 7, "sha256": hashlib.sha256(b"fixture").hexdigest()}
        with patch.object(provision.urllib.request, "urlopen", return_value=io.BytesIO(b"fixture")):
            provision.download("https://example.invalid/public", path, record)
        self.assertEqual(path.read_bytes(), b"fixture")
        self.assertEqual(partial.read_bytes(), b"another transfer")

    def test_worker_rejects_extra_extension_before_first_package_import(self):
        original = Path(bundle.__file__).resolve().parent.parent
        worker = self.root / "worker.py"
        shutil.copyfile(original / "worker.py", worker)
        package = self.root / "mas_privacy_opf"; package.mkdir()
        marker = self.root / "imported.txt"
        modules = ("__init__.py", "contracts.py", "transport.py", "bundle.py", "inference.py", "service.py")
        names = []
        for name in modules:
            target = package / name
            target.write_text("from pathlib import Path\nPath(" + repr(str(marker)) + ").write_text('imported')\n" if name == "__init__.py" else "")
            names.append("mas_privacy_opf/" + name)
        for name in ("upstream-code.lock.json", "upstream-assets.lock.json", "runtime.lock.json"):
            (self.root / name).write_text("{}")
            names.append(name)
        manifest = {"worker_sha256": bundle.hash_file(worker), "adapter_files": {
            name: {"bytes": (self.root / name).stat().st_size, "sha256": bundle.hash_file(self.root / name)} for name in names}}
        path = self.root / "manifest.json"; path.write_text(json.dumps(manifest))
        command = [sys.executable, "-I", str(worker), "--manifest", str(path), "--assets", str(self.root)]
        baseline = subprocess.run(command, capture_output=True, timeout=10)
        self.assertEqual(baseline.returncode, 1)
        self.assertTrue(marker.exists())
        marker.unlink()
        extension = package / "__init__.cpython-312-x86_64-linux-gnu.so"
        extension.write_bytes(b"unreviewed native code")
        result = subprocess.run(command, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 1)
        self.assertFalse(marker.exists())
        self.assertEqual((result.stdout, result.stderr), (b"", b""))


class FailureAndTransport(unittest.TestCase):
    def test_asset_and_model_exception_text_never_leaves_response(self):
        value = fixture("synthetic private payload")
        with patch.object(service, "read_manifest", side_effect=ValueError(value["text"])):
            result = service.assess(value, "unused", "unused")
        self.assertEqual(result["completion"], "failed")
        self.assertNotIn(value["text"], json.dumps(result))
        self.assertTrue(result["omissions"])

    def test_private_fd_roundtrip_and_oversized_prefix(self):
        left, right = socket.socketpair(); self.addCleanup(left.close); self.addCleanup(right.close)
        thread = threading.Thread(target=lambda: write_frame(left.fileno(), b'{"safe":true}'))
        thread.start(); self.assertEqual(read_frame(right.fileno()), b'{"safe":true}'); thread.join()
        os.write(left.fileno(), (65537).to_bytes(4, "big"))
        with self.assertRaises(AdapterError): read_frame(right.fileno())


if __name__ == "__main__":
    unittest.main()
