"""The Python application facade uses the host CLI and stdin object input."""

from dataclasses import dataclass
import json
import subprocess
import unittest
from unittest.mock import patch

from mas_host import Client, HostError
from mas_host.codec import dumps_object, loads_object


@dataclass(frozen=True)
class Input:
    quantity: int


@dataclass(frozen=True)
class Output:
    total: int


class ClientTests(unittest.TestCase):
    def test_project_run_serialises_dataclass_on_stdin_and_decodes_output(self):
        completed = subprocess.CompletedProcess([], 0, b'{"status":"completed","run_id":"trial","output":{"total":12}}', b"")
        with patch("subprocess.run", return_value=completed) as run:
            result = Client(host="custom-mas-host", project="my project", state="my state", run_id="trial").run("counter", Input(6), output_type=Output)
        self.assertEqual(result, Output(12))
        args, kwargs = run.call_args
        argv = args[0]
        self.assertIsInstance(argv, list)
        self.assertEqual(argv[0], "custom-mas-host")
        self.assertIn("run", argv)
        for flag, value in (("--project", "my project"), ("--agent", "counter"), ("--input", "-"), ("--state", "my state"), ("--run", "trial")):
            self.assertEqual(argv[argv.index(flag) + 1], value)
        self.assertEqual(loads_object(kwargs["input"]), {"quantity": 6})
        self.assertEqual(kwargs["stdout"], subprocess.PIPE)
        self.assertEqual(kwargs["stderr"], subprocess.PIPE)
        self.assertFalse(kwargs.get("shell", False))
        self.assertEqual(run.call_count, 1)

    def test_existing_bundle_run_avoids_project_build_arguments(self):
        completed = subprocess.CompletedProcess([], 0, b'{"status":"completed","run_id":"r","output":{"n":1}}', b"")
        with patch("subprocess.run", return_value=completed) as run:
            self.assertEqual(Client(bundle="bundle.json", run_id="r").run("worker", {}), {"n": 1})
        argv = run.call_args.args[0]
        self.assertEqual(argv[argv.index("--bundle") + 1], "bundle.json")
        self.assertNotIn("--project", argv)

    def test_pretty_cli_envelope_can_exceed_native_frame_size(self):
        payload = {"items": list(range(12000))}
        native = dumps_object(payload)
        envelope = json.dumps({"status": "completed", "run_id": "trial", "output": payload}, indent=2).encode("utf-8")
        self.assertLess(len(native), 65536)
        self.assertGreater(len(envelope), 65536)
        completed = subprocess.CompletedProcess([], 0, envelope, b"")
        with patch("subprocess.run", return_value=completed) as run:
            result = Client(bundle="bundle.json", run_id="trial").run("worker", payload)
        self.assertEqual(result, payload)
        self.assertEqual(run.call_args.kwargs["input"], native)
        self.assertEqual(run.call_count, 1)

    def test_completion_requires_the_requested_run_identity(self):
        for response in (
            {"status": "completed", "output": {"total": 12}},
            {"status": "completed", "run_id": "another-run", "output": {"total": 12}},
        ):
            raw = json.dumps(response).encode("utf-8")
            with self.subTest(response=response), patch("subprocess.run", return_value=subprocess.CompletedProcess([], 0, raw, b"")) as run:
                with self.assertRaises(HostError):
                    Client(bundle="bundle.json", run_id="trial").run("worker", {})
            self.assertEqual(run.call_count, 1)

    def test_host_failure_surfaces_stderr_without_retry(self):
        completed = subprocess.CompletedProcess([], 1, b"", b'{"status":"error","error":"effect requires reconciliation"}')
        with patch("subprocess.run", return_value=completed) as run:
            with self.assertRaisesRegex(HostError, "reconciliation"):
                Client(bundle="bundle.json").run("writer", {})
        self.assertEqual(run.call_count, 1)

    def test_malformed_success_response_is_rejected(self):
        for raw in (b"not json", b'{"status":"completed","run_id":"trial"}', b'{"status":"completed","run_id":"trial","output":[]}', b'{"status":"completed","run_id":"trial","output":{"n":1.5}}'):
            with self.subTest(raw=raw), patch("subprocess.run", return_value=subprocess.CompletedProcess([], 0, raw, b"")):
                with self.assertRaises((HostError, ValueError, TypeError)):
                    Client(bundle="bundle.json", run_id="trial").run("worker", {})

    def test_invalid_input_never_launches_host(self):
        with patch("subprocess.run") as run:
            with self.assertRaises((TypeError, ValueError)):
                Client(project=".").run("worker", {"float": 0.5})
        run.assert_not_called()

    def test_requires_exactly_one_project_or_bundle(self):
        for arguments in ({}, {"project": ".", "bundle": "bundle.json"}):
            with self.subTest(arguments=arguments), self.assertRaises((TypeError, ValueError)):
                Client(**arguments)


if __name__ == "__main__":
    unittest.main()
