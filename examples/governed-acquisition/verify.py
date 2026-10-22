"""Exercise acquisition, inherited restrictions and read-only restart recovery."""
import argparse
import http.client
import json
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import time

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "sdk/python"))
from application import POINT, PRIVATE
from prepare import prepare


TOKEN = "synthetic-acquisition-local-token-0123456789"
BASE = "urn:mas:host-task-service:1"
SOURCE = "urn:mas:host-governed-source-read:1"
EFFECT = "urn:mas:host-governed-effect-read:1"


def checked(host, *args):
    result = subprocess.run([host, *map(str, args)], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


class Service:
    def __init__(self, host, project, port, *, state="state"):
        self.port = port
        self.process = subprocess.Popen([host, "serve-a2a", "--bundle", str(project / ".mas/build/bundle.json"),
            "--state", str(project / state), "--config", str(project / "gateway.json")],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        deadline = time.monotonic() + 30
        while True:
            assert self.process.poll() is None, self.process.stderr.read().decode()
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                    return
            except OSError:
                assert time.monotonic() < deadline, "service-startup-timeout"
                time.sleep(0.02)

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        raw = self.process.stderr.read()
        assert PRIVATE.encode() not in raw
        self.process.stderr.close()

    def rpc(self, method, params, *, extension=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=150)
        try:
            body = json.dumps({"jsonrpc": "2.0", "id": "synthetic-safe-id", "method": method, "params": params})
            extensions = BASE if extension is None else BASE + "," + extension
            connection.request("POST", "/a2a", body=body.encode(), headers={"Content-Type": "application/json",
                "Authorization": "Bearer " + TOKEN, "A2A-Version": "1.0", "A2A-Extensions": extensions})
            raw = connection.getresponse().read(65537)
            assert len(raw) <= 65536 and PRIVATE.encode() not in raw
            return json.loads(raw)
        finally:
            connection.close()

    def terminal(self, identity):
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            result = self.rpc("GetTask", {"id": identity})
            if result.get("error") == {"code": -32001, "message": "task_not_found"}:
                # The full closure may be unavailable while acquisition is
                # pending. Retry the original query within this bound without
                # treating a uniform lookup failure as an execution state.
                time.sleep(0.05)
                continue
            assert "error" not in result, result
            task = result["result"]
            state = task["metadata"]["mas"]["state"]
            if state == "completed":
                return task
            assert state not in ("failed", "unknown"), task
            time.sleep(0.05)
        raise AssertionError("task-completion-timeout")

    def retained(self, identity, method, extension):
        response = self.rpc(method, {"id": identity, "key": "acquire"}, extension=extension)
        assert "error" not in response, response
        return response["result"]


def calls(project, name):
    path = project / name
    return len(path.read_bytes()) if path.exists() else 0


def count_contract(project, count=1):
    assert calls(project, "collector-calls") == count
    assert calls(project, "blocked-action-calls") == calls(project, "blocked-peer-calls") == count
    assert calls(project, "permitted-peer-calls") == count
    assert (project / "consumed-ready-replicas").read_text() == "2"


def no_original(project):
    assert (project / "original-inspected").exists()
    assert calls(project, "inspection-calls") > 0
    for state in ("state", "baseline-source-state"):
        for path in (project / state).rglob("*"):
            if path.is_file():
                assert PRIVATE.encode() not in path.read_bytes(), path


def request(descriptor, mode):
    return {"message": {"messageId": "synthetic-acquisition-" + mode, "role": "ROLE_USER", "parts": [
        {"data": {"agent": "collector", "descriptorDigest": descriptor, "input": {"mode": mode}},
         "mediaType": "application/json"}]}, "configuration": {"returnImmediately": True}}


def output(task):
    return task["artifacts"][0]["parts"][0]["data"]


def source_contract(source, effect):
    """Check the approved source against its original completed effect."""
    assert set(source) == {"version", "namespace_ref", "actor", "parent_invocation", "key", "effect_id",
                           "source_id", "state", "host_settlement", "external_execution", "payload_disposition",
                           "source_metadata", "lineage", "projection", "raw_source_disposition"}
    assert source["version"] == "mas-governed-source-view/1"
    for field in ("namespace_ref", "actor", "parent_invocation", "key", "effect_id",
                  "state", "host_settlement", "external_execution", "payload_disposition"):
        assert source[field] == effect[field], field
    assert re.fullmatch(r"[0-9a-f]{64}", source["namespace_ref"])
    assert re.fullmatch(r"task-[0-9a-f]{64}", source["parent_invocation"])
    assert re.fullmatch(r"eff-[0-9a-f]{64}", source["effect_id"])
    assert re.fullmatch(r"source-[0-9a-f]{64}", source["source_id"])
    assert source["projection"] == POINT
    assert source["raw_source_disposition"] == "not-retained"
    metadata = source["source_metadata"]
    assert metadata["id"] == source["source_id"] and metadata["step_id"] == source["effect_id"]
    assert metadata["kind"] == "effect-output" and metadata["actor"] == "collector"
    assert metadata["policy_refs"] == ["baseline", "source-restrictions"]
    lineage = source["lineage"]
    assert type(lineage) is list and 1 <= len(lineage) <= 64
    indexed = {item["id"]: item for item in lineage}
    assert len(indexed) == len(lineage) and indexed[source["source_id"]] == metadata
    for item in lineage:
        assert set(item) == {"version", "id", "step_id", "kind", "actor", "payload_binding", "policy_refs",
                             "parents", "acquired_at_ms", "expires_at_ms", "coverage", "classification",
                             "payload_available"}
        assert item["version"] == "mas-governed-source/1"
        assert re.fullmatch(r"source-[0-9a-f]{64}", item["id"])
        assert re.fullmatch(r"[0-9a-f]{64}", item["payload_binding"])
        assert item["coverage"] == "complete" and item["classification"] == "restricted"
        assert item["payload_available"] is True
        assert "baseline" in item["policy_refs"]
        assert set(item["parents"]) <= set(indexed)
        assert type(item["acquired_at_ms"]) is int and type(item["expires_at_ms"]) is int
        assert 0 < item["acquired_at_ms"] < item["expires_at_ms"]
    assert metadata["parents"]  # The acquisition preserves its admitted input ancestry.
    assert effect["version"] == "mas-governed-effect-view/1"
    assert effect["key"] == "acquire" and effect["actor"] == "collector"
    assert effect["state"] == effect["host_settlement"] == "completed"
    assert effect["external_execution"] == "completed" and effect["payload_disposition"] == "available"
    assert effect["descriptor"] == {"format": "fixture-deployment-read/1"}
    assert effect["output"] == POINT
    receipt = effect["evidence"]["inspection"]
    assert receipt["original_findings_count"] > 0 and receipt["projected_findings_count"] == 0
    assert PRIVATE not in json.dumps(source)


def run(host, fixture_python):
    with tempfile.TemporaryDirectory(prefix="mas-governed-acquisition-") as temporary:
        project = prepare(Path(temporary) / "application", fixture_python)
        checked(host, "build", "--project", project)
        descriptor = checked(host, "describe", "--bundle", project / ".mas/build/bundle.json")["bundle_digest"]
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        gateway = json.loads((project / "gateway.json").read_text())
        gateway.update(listen=f"127.0.0.1:{port}", public_url=f"http://127.0.0.1:{port}/a2a")
        (project / "gateway.json").write_text(json.dumps(gateway))
        service = Service(host, project, port)
        original = request(descriptor, "acquire")
        try:
            baseline = service.rpc("SendMessage", request(descriptor, "baseline"))
            assert "error" not in baseline, baseline
            control = service.terminal(baseline["result"]["task"]["id"])
            assert output(control) == {"acquired": False, "action_denied": False, "peer_denied": False,
                                       "consumer_accepted": False}, control
            assert calls(project, "collector-calls") == 0
            assert calls(project, "blocked-action-calls") == calls(project, "blocked-peer-calls") == 1
            acquired = service.rpc("SendMessage", original)
            assert "error" not in acquired, acquired
            identity = acquired["result"]["task"]["id"]
            task = service.terminal(identity)
            assert output(task) == {"acquired": True, "action_denied": True, "peer_denied": True,
                                    "consumer_accepted": True}, task
            count_contract(project)
            missing_extension = service.rpc("GetRetainedSource", {"id": identity, "key": "acquire"})
            assert missing_extension["error"]["message"] == "required_source_extension_missing"
            source = service.retained(identity, "GetRetainedSource", SOURCE)
            effect = service.retained(identity, "GetRetainedEffect", EFFECT)
            source_contract(source, effect)
            no_original(project)
        finally:
            service.close()
        restarted = Service(host, project, port)
        try:
            replay = restarted.rpc("SendMessage", original)
            assert replay["result"]["task"]["artifacts"] == task["artifacts"], replay
            retained_source = restarted.retained(identity, "GetRetainedSource", SOURCE)
            retained_effect = restarted.retained(identity, "GetRetainedEffect", EFFECT)
            source_contract(retained_source, retained_effect)
            assert retained_source == source and retained_effect == effect
            count_contract(project)
            no_original(project)
            authority = json.loads((project / "authority.json").read_text())
            active_grant = authority["grants"][0]
            authority["grants"] = []
            (project / "authority-next.json").write_text(json.dumps(authority))
            (project / "authority-next.json").replace(project / "authority.json")
            for method, params, extension in (("GetRetainedSource", {"id": identity, "key": "acquire"}, SOURCE),
                                              ("GetRetainedEffect", {"id": identity, "key": "acquire"}, EFFECT),
                                              ("GetTask", {"id": identity}, None), ("SendMessage", original, None)):
                assert "error" in restarted.rpc(method, params, extension=extension), method
            count_contract(project)
            no_original(project)
        finally:
            restarted.close()
        # Keep the exact bundle, acquire-mode code, schemas and detector assets.
        # Select baseline-only source restrictions before opening a fresh state
        # namespace. The original restricted namespace is never reopened with
        # changed policy configuration.
        authority["grants"] = [active_grant]
        (project / "authority.json").write_text(json.dumps(authority))
        governance = json.loads((project / "governance.json").read_text())
        governance["agents"][0]["effects"][0]["source_policy_refs"] = ["baseline"]
        (project / "governance.json").write_text(json.dumps(governance))
        control_service = Service(host, project, port, state="baseline-source-state")
        try:
            same_acquire = control_service.rpc("SendMessage", original)
            assert "error" not in same_acquire, same_acquire
            task = control_service.terminal(same_acquire["result"]["task"]["id"])
            assert output(task) == {"acquired": True, "action_denied": False, "peer_denied": False,
                                    "consumer_accepted": True}, task
            count_contract(project, 2)
            no_original(project)
        finally:
            control_service.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", required=True)
    parser.add_argument("--fixture-python", default="/usr/bin/python3")
    args = parser.parse_args()
    run(str(Path(args.host).resolve()), args.fixture_python)
    print(json.dumps({"scenario": "governed-acquisition", "status": "PASS"}))
