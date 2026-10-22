"""Run one actual-host synthetic detector/response/recovery conformance scenario."""
import argparse
import http.client
import json
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
import sys

from prepare import prepare

PRIVATE = b"private-response-person-8731@example.invalid"
TOKEN = "synthetic-response-local-token-0123456789"
COMPLETION_SECONDS = 240


def checked(host, *args):
    result = subprocess.run([host, *map(str, args)], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


class Service:
    def __init__(self, host, project, port):
        self.port = port
        self.process = subprocess.Popen([host, "serve-a2a", "--bundle", str(project / ".mas/build/bundle.json"),
            "--state", str(project / "state"), "--config", str(project / "gateway.json")],
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
        assert PRIVATE not in raw
        self.process.stderr.close()

    def rpc(self, method, params):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=40)
        try:
            body = json.dumps({"jsonrpc": "2.0", "id": "synthetic-safe-id", "method": method, "params": params})
            connection.request("POST", "/a2a", body=body.encode(), headers={"Content-Type": "application/json",
                "Authorization": "Bearer " + TOKEN, "A2A-Version": "1.0", "A2A-Extensions": "urn:mas:host-task-service:1"})
            raw = connection.getresponse().read(65537)
            assert len(raw) <= 65536 and PRIVATE not in raw
            return json.loads(raw)
        finally:
            connection.close()

    def terminal(self, identity, expected):
        deadline = time.monotonic() + COMPLETION_SECONDS
        while time.monotonic() < deadline:
            result = self.rpc("GetTask", {"id": identity})
            assert "error" not in result, result
            task = result["result"]
            state = task["metadata"]["mas"]["state"]
            if state == expected:
                return task
            assert state not in ("completed", "failed", "unknown"), task
            time.sleep(0.05)
        raise AssertionError("task-completion-timeout")


def journal(project):
    files = list((project / "state").rglob("execution.sqlite"))
    assert len(files) == 1, files
    return files[0]


def states(project):
    with sqlite3.connect(journal(project)) as db:
        values = db.execute("SELECT input,state FROM steps WHERE kind='effect' ORDER BY rowid").fetchall()
    names = {"synthetic-private-collection/1": "collect", "mas-privacy-case-intent/1": "privacy-case-intent",
             "mas-privacy-notification-submit/1": "privacy-notification"}
    return [(names[json.loads(value)["format"]], state) for value, state in values]


def count(project, name):
    path = project / name
    return len(path.read_bytes()) if path.exists() else 0


def private_absent(project):
    for path in (project / "state").rglob("*"):
        if path.is_file():
            assert PRIVATE not in path.read_bytes(), path
    if (project / "notification.json").exists():
        raw = (project / "notification.json").read_bytes()
        assert PRIVATE not in raw
        value = json.loads(raw)
        assert value["reason"] == "handling-withheld"
        assert value["categories"] == [] and value["finding_count"] == 0
    assert (project / "private-seen").exists()
    assert count(project, "inspection-calls") > 0


def run(host, mode, fixture_python="/usr/bin/python3"):
    with tempfile.TemporaryDirectory(prefix="mas-privacy-response-") as temporary:
        project = prepare(Path(temporary) / "application", fixture_python)
        checked(host, "build", "--project", project)
        descriptor = checked(host, "describe", "--bundle", project / ".mas/build/bundle.json")["bundle_digest"]
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        gateway = json.loads((project / "gateway.json").read_text())
        gateway.update(listen=f"127.0.0.1:{port}", public_url=f"http://127.0.0.1:{port}/a2a")
        (project / "gateway.json").write_text(json.dumps(gateway))
        request = {"message": {"messageId": "synthetic-original-event", "role": "ROLE_USER", "parts": [
            {"data": {"agent": "responder", "descriptorDigest": descriptor,
                       "input": {"mode": mode, "discarded_private": PRIVATE.decode()}}, "mediaType": "application/json"}]},
            "configuration": {"returnImmediately": True}}
        service = Service(host, project, port)
        try:
            submitted = service.rpc("SendMessage", request)
            assert "error" not in submitted, submitted
            identity = submitted["result"]["task"]["id"]
            if mode == "revoke-before-notify":
                deadline = time.monotonic() + COMPLETION_SECONDS
                while not (project / "case-ready").exists():
                    assert time.monotonic() < deadline, states(project)
                    time.sleep(0.02)
                authority = json.loads((project / "authority.json").read_text())
                authority["grants"][0]["status"] = "revoked"
                (project / "authority-next.json").write_text(json.dumps(authority))
                (project / "authority-next.json").replace(project / "authority.json")
                (project / "continue").write_text("continue")
                deadline = time.monotonic() + 20
                while True:
                    with sqlite3.connect(journal(project)) as db:
                        active = db.execute("SELECT count(*) FROM steps WHERE state='running'").fetchone()[0]
                    if not active:
                        break
                    assert time.monotonic() < deadline, "revocation-settlement-timeout"
                    time.sleep(0.02)
                assert count(project, "notification-calls") == 0
                assert states(project) == [("collect", "completed"), ("privacy-case-intent", "completed")]
                assert "error" in service.rpc("GetTask", {"id": identity})
                private_absent(project)
                return
            try:
                task = service.terminal(identity, "unknown" if mode == "unknown" else "completed")
            except AssertionError:
                print(json.dumps({"committed_effect_states": states(project)}), file=sys.stderr)
                raise
            if mode != "unknown":
                output = task["artifacts"][0]["parts"][0]["data"]
                assert output["submission"] == mode, task
                assert output["recipient_delivery"] == "unknown"
                assert output["retry_after_seconds"] == (60 if mode == "throttled" else 0)
            assert count(project, "collector-calls") == count(project, "notification-calls") == 1
            observed = states(project)
            assert observed[:2] == [("collect", "completed"), ("privacy-case-intent", "completed")], observed
            assert observed[2][0] == "privacy-notification"
            assert observed[2][1] in (("executing", "unknown") if mode == "unknown" else ("completed",))
            retained = (project / "notification.json").read_bytes()
            private_absent(project)
        finally:
            service.close()
        restarted = Service(host, project, port)
        try:
            replay = restarted.rpc("SendMessage", request)
            if mode == "unknown":
                assert "error" in replay or replay["result"]["task"]["metadata"]["mas"]["state"] == "unknown", replay
                assert states(project)[2] == ("privacy-notification", "unknown")
            else:
                assert replay["result"]["task"]["artifacts"] == task["artifacts"], replay
            assert count(project, "collector-calls") == count(project, "notification-calls") == 1
            assert (project / "notification.json").read_bytes() == retained
            changed = json.loads(json.dumps(request))
            changed["message"]["parts"][0]["data"]["input"]["discarded_private"] = "changed-original-private"
            assert "error" in restarted.rpc("SendMessage", changed)
            private_absent(project)
        finally:
            restarted.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", required=True)
    parser.add_argument("--fixture-python", default="/usr/bin/python3")
    parser.add_argument("--scenario", choices=("accepted", "throttled", "unknown", "revoke-before-notify"), required=True)
    args = parser.parse_args()
    run(str(Path(args.host).resolve()), args.scenario, args.fixture_python)
    print(json.dumps({"scenario": args.scenario, "status": "PASS"}))
