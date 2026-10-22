#!/usr/bin/env python3
"""Exercise provisioned N1 assets through the governed A2A2 path, without downloads."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import time


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def write(path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", type=Path, required=True)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.root.mkdir()
    root = args.root.resolve()
    host = args.host.resolve()
    manifest_bytes = args.manifest.read_bytes()
    manifest = json.loads(manifest_bytes)
    started = time.monotonic()
    result = {
        "version": "mas-governed-neural-smoke/1",
        "mode": "actual-pinned-local-model-synthetic-integer-observation",
        "host_sha256": digest(host.read_bytes()),
        "script_sha256": digest(Path(__file__).read_bytes()),
        "manifest_sha256": digest(manifest_bytes),
        "detector": {name: manifest[name] for name in ("code_revision", "model_revision", "backend", "adapter")},
        "quality_qualified": False,
        "checks": [],
    }
    result["detector"]["bundle_sha256"] = result["manifest_sha256"]
    schema = {"type": "object", "properties": {"count": {"type": "integer", "minimum": 0, "maximum": 10}}, "required": ["count"], "additionalProperties": False}
    write(root / "input.json", schema)
    write(root / "output.json", schema)
    (root / "application.py").write_text("def collect(ctx,value):\n    with open('calls','a') as stream: stream.write('x')\n    return value\n")
    (root / "agents.toml").write_text('version = "mas-agents/3"\n[[agents]]\nname = "collector"\noperation = "count/1"\ninput_schema = "input.json"\noutput_schema = "output.json"\n[agents.implementation]\nkind = "native"\nlanguage = "python"\nsource = "application.py"\nhandler = "collect"\n[limits]\nwall_seconds = 600\nmax_calls = 8\n')
    write(root / "topology.json", {"version": "mas-topology/1", "edges": [["client", "collector"], ["collector", "client"]]})
    build = subprocess.run([str(host), "build", "--project", str(root)], capture_output=True, check=True)
    (root / "build.stdout").write_bytes(build.stdout)
    (root / "build.stderr").write_bytes(build.stderr)
    bundle = Path(json.loads(build.stdout)["bundle"])
    descriptor = json.loads(subprocess.check_output([str(host), "describe", "--bundle", str(bundle)]))["bundle_digest"]
    key = root / "commitment.key"
    key.write_bytes(os.urandom(32))
    key.chmod(0o600)
    expiry = int(time.time() * 1000) + 1_800_000
    token = "actual-neural-synthetic-local-token-000001"
    write(root / "authentication.json", {"version": "mas-host-authentication/1", "principals": [{"tenant": "synthetic-tenant", "principal": "smoke", "purpose": "qualification", "grant_ref": "synthetic-grant", "token_sha256": digest(token.encode()), "agents": ["collector"]}]})
    rules = []
    for operation, recipient, destination in (("invoke", "worker:collector", "local-worker"), ("deliver", "smoke", "client"), ("inspect", "privacy-inspector", "local-detector")):
        rules.append({"operation": operation, "principal": "smoke", "actor": "collector", "producer": "host", "recipient": recipient, "purpose": "qualification", "destination": destination, "location": "local", "classifications": ["restricted"], "obligations": ["inline-retention-approved"], "max_bytes": 16384})
    write(root / "authority.json", {"version": "mas-governed-authority/1", "grants": [{"version": "mas-current-authority/1", "status": "active", "tenant": "synthetic-tenant", "principal": "smoke", "grant_ref": "synthetic-grant", "grant_epoch": 1, "policy_version": "synthetic-p1", "valid_from_ms": 0, "expires_at_ms": expiry}], "policies": [{"version": "mas-data-use-policy/1", "id": "synthetic-policy", "tenant": "synthetic-tenant", "policy_version": "synthetic-p1", "valid_from_ms": 0, "expires_at_ms": expiry, "rules": rules}]})
    write(root / "governance.json", {"version": "mas-governed-local/1", "authority_file": str(root / "authority.json"), "commitment_key_file": str(key), "commitment_key_id": "synthetic-key-1", "location": "local", "agents": [{"actor": "collector", "destination": "local-worker", "input_schema": schema, "output_schema": schema, "effects": []}]})
    write(root / "privacy.json", {"version": "mas-governed-privacy-inspector/1", "python": str(args.python.absolute()), "worker": str(args.repo.resolve() / "adapters/privacy/worker.py"), "manifest": str(args.manifest.resolve()), "assets": str(args.assets.resolve()), "detector": result["detector"], "python_sha256": digest(args.python.read_bytes()), "destination": "local-detector", "timeout_ms": 120000, "memory_mib": 12288})
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    write(root / "gateway.json", {"version": "mas-host-a2a/2", "listen": f"127.0.0.1:{port}", "public_url": f"http://127.0.0.1:{port}/a2a", "name": "Actual neural synthetic smoke", "governance_file": str(root / "governance.json"), "authentication_file": str(root / "authentication.json"), "privacy_inspector_file": str(root / "privacy.json"), "exports": [{"agent": "collector", "name": "Counter", "description": "Synthetic bounded integer observation", "public": False}]})
    server = None
    try:
        with (root / "service.stdout").open("wb") as stdout, (root / "service.stderr").open("wb") as stderr:
            server = subprocess.Popen([str(host), "serve-a2a", "--bundle", str(bundle), "--state", str(root / "state"), "--config", str(root / "gateway.json")], stdout=stdout, stderr=stderr)
        ready_deadline = time.monotonic() + 30
        while True:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=1):
                    break
            except OSError:
                if server.poll() is not None or time.monotonic() >= ready_deadline:
                    raise RuntimeError("service-startup-failed")
                time.sleep(0.05)

        def rpc(method, params):
            began = time.monotonic()
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=270)
            body = json.dumps({"jsonrpc": "2.0", "id": "safe-smoke-request", "method": method, "params": params})
            connection.request("POST", "/a2a", body, {"Content-Type": "application/json", "Authorization": "Bearer " + token, "A2A-Version": "1.0", "A2A-Extensions": "urn:mas:host-task-service:1"})
            response = connection.getresponse()
            value = json.loads(response.read())
            result["checks"].append({"method": method, "seconds": round(time.monotonic() - began, 4), "http_status": response.status, "response": value})
            connection.close()
            return value

        request = {"message": {"messageId": "actual-neural-count-1", "role": "ROLE_USER", "parts": [{"mediaType": "application/json", "data": {"agent": "collector", "descriptorDigest": descriptor, "input": {"count": 1}}}]}, "configuration": {"returnImmediately": True}}
        rpc("SendMessage", request)
        db = sqlite3.connect(root / "state/service.sqlite")
        deadline = time.monotonic() + 180
        rows = []
        while time.monotonic() < deadline:
            rows = db.execute("select id,state,output,error from tasks").fetchall()
            if not rows or rows[0][1] in ("completed", "failed", "unknown"):
                break
            time.sleep(0.1)
        result["retained_tasks"] = [{"id": row[0], "state": row[1], "output": json.loads(row[2]) if row[2] else None, "error": row[3]} for row in rows]
        if rows and rows[0][1] == "completed":
            rpc("GetTask", {"id": rows[0][0]})
            rpc("SendMessage", request)
        result["handler_dispatches"] = len((root / "calls").read_bytes()) if (root / "calls").exists() else 0
        result["status"] = "PASS" if rows and rows[0][1] == "completed" and result["handler_dispatches"] == 1 and all("error" not in check["response"] for check in result["checks"]) else "INCOMPLETE_OR_WITHHELD"
    except Exception as error:
        result["status"] = "ERROR"
        result["error_type"] = type(error).__name__
        result["error"] = str(error)
    finally:
        if server is not None:
            server.terminate()
            try:
                server.wait(timeout=15)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
        result["seconds"] = round(time.monotonic() - started, 4)
        write(args.output, result)
        print(json.dumps({"status": result["status"], "seconds": result["seconds"], "output": str(args.output)}))


if __name__ == "__main__":
    main()
