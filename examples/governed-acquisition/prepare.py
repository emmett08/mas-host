"""Prepare a bounded trusted-local source acquisition and exposure fixture."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


ROOT = Path(__file__).resolve().parent


def record(properties):
    return {"type": "object", "properties": properties, "required": list(properties),
            "additionalProperties": False}


def enum(*values):
    return {"type": "string", "enum": list(values)}


def prepare(destination, fixture_python="/usr/bin/python3"):
    destination = Path(destination).resolve()
    destination.mkdir(parents=True, exist_ok=False)
    for name in ("application.py", "fixture_inspector.py"):
        shutil.copyfile(ROOT / name, destination / name)
    input_schema = record({"mode": enum("baseline", "acquire")})
    output_schema = record({name: {"type": "boolean"} for name in
                            ("acquired", "action_denied", "peer_denied", "consumer_accepted")})
    peer_input = record({"ready_replicas": {"type": "integer", "minimum": 0, "maximum": 100}})
    peer_output = record({"accepted": {"type": "boolean"}})
    point = record({"version": enum("governed-deployment-point/1"), "cluster_uid": enum("fixture-cluster-001"),
                    "namespace_uid": enum("fixture-namespace-001"), "namespace": enum("demo"),
                    "kind": enum("Deployment"), "name": enum("web"), "uid": enum("fixture-deployment-001"),
                    "resource_version": enum("17"), "generation": {"type": "integer", "minimum": 1, "maximum": 2147483647},
                    "desired_replicas": {"type": "integer", "minimum": 0, "maximum": 100},
                    "ready_replicas": {"type": "integer", "minimum": 0, "maximum": 100},
                    "freshness": enum("NOT_RUN"), "assessment": enum("NOT_RUN")})
    evidence = record({"execution": enum("completed")})
    effects = [
        {"format": "fixture-deployment-read/1", "destination": "fixture-source",
         "source_policy_refs": ["source-restrictions"],
         "descriptor_schema": record({"format": enum("fixture-deployment-read/1")}),
         "output_schema": point, "evidence_schema": evidence},
        {"format": "fixture-blocked-action/1", "destination": "fixture-external-action",
         "descriptor_schema": record({"format": enum("fixture-blocked-action/1")}),
         "output_schema": record({"performed": {"type": "boolean"}}), "evidence_schema": evidence},
    ]
    agents = [{"actor": "collector", "destination": "local-worker", "input_schema": input_schema,
               "output_schema": output_schema, "effects": effects}]
    agents += [{"actor": name, "destination": "local-worker", "input_schema": peer_input,
                "output_schema": peer_output, "effects": []} for name in ("permitted", "blocked")]
    governance = {"version": "mas-governed-local/2", "baseline_policy_refs": ["baseline"],
                  "authority_file": str(destination / "authority.json"),
                  "commitment_key_file": str(destination / "commitment.key"),
                  "commitment_key_id": "synthetic-acquisition-key-1", "location": "local", "agents": agents}
    rules = []
    for actor in ("collector", "permitted", "blocked"):
        bindings = [("invoke", "worker:" + actor, "local-worker"),
                    ("deliver", "synthetic-user", "client"),
                    ("inspect", "privacy-inspector", "local-detector"),
                    ("resolve", "synthetic-user", "client")]
        if actor == "collector":
            bindings += [("invoke", "worker:" + peer, "local-worker") for peer in ("permitted", "blocked")]
            bindings += [("effect", "operation:" + effect["format"], effect["destination"]) for effect in effects]
        else:
            bindings.append(("invoke", "worker:collector", "local-worker"))
        for operation, recipient, target in bindings:
            rules.append({"operation": operation, "principal": "synthetic-user", "actor": actor,
                          "producer": "host", "recipient": recipient, "purpose": "synthetic-acquisition",
                          "destination": target, "location": "local", "classifications": ["restricted"],
                          "obligations": ["inline-retention-approved"], "max_bytes": 65536})
    policy = {"version": "mas-data-use-policy/1", "id": "baseline", "tenant": "synthetic-tenant",
              "policy_version": "1", "valid_from_ms": 0, "expires_at_ms": 4102444800000, "rules": rules}
    source_rules = [rule for rule in rules if rule["actor"] != "blocked" and
                    rule["recipient"] not in ("worker:blocked", "operation:fixture-blocked-action/1")]
    restricted_policy = {**policy, "id": "source-restrictions", "rules": source_rules}
    authority = {"version": "mas-governed-authority/1", "grants": [
        {"version": "mas-current-authority/1", "status": "active", "tenant": "synthetic-tenant",
         "principal": "synthetic-user", "grant_ref": "synthetic-grant", "grant_epoch": 1,
         "policy_version": "1", "valid_from_ms": 0, "expires_at_ms": 4102444800000}],
        "policies": [policy, restricted_policy]}
    python = Path(fixture_python)
    if not python.is_absolute():
        raise ValueError("synthetic-fixture-python-requires-absolute-path")
    python = python.resolve(strict=True)
    checked = subprocess.run([str(python), "-I", "-c", "import sys; assert sys.version_info >= (3, 11)"],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
    if checked.returncode:
        raise ValueError("synthetic-fixture-python-requires-3.11-or-later")
    worker = (destination / "fixture_inspector.py").read_bytes()
    manifest = {"version": "mas-privacy-bundle/1", "code_revision": "a" * 40, "model_revision": "b" * 40,
                "backend": "opf-cpu-bfloat16/1", "adapter": "mas-privacy-opf/1",
                "worker_sha256": hashlib.sha256(worker).hexdigest(), "adapter_files": {}, "files": {}, "runtime": {},
                "settings": {"max_input_bytes": 16384, "max_response_bytes": 65536, "max_findings": 128}}
    raw = json.dumps(manifest, separators=(",", ":"), sort_keys=True).encode()
    (destination / "inspection-manifest.json").write_bytes(raw)
    inspector = {"version": "mas-governed-privacy-inspector/1", "python": str(python),
                 "python_sha256": hashlib.sha256(python.read_bytes()).hexdigest(),
                 "worker": str(destination / "fixture_inspector.py"), "manifest": str(destination / "inspection-manifest.json"),
                 "assets": str(destination), "detector": {"bundle_sha256": hashlib.sha256(raw).hexdigest(),
                 "code_revision": "a" * 40, "model_revision": "b" * 40,
                 "backend": "opf-cpu-bfloat16/1", "adapter": "mas-privacy-opf/1"},
                 "destination": "local-detector", "timeout_ms": 60000, "memory_mib": 256}
    gateway = {"version": "mas-host-a2a/2", "listen": "127.0.0.1:8787", "public_url": "http://127.0.0.1:8787/a2a",
               "name": "Synthetic governed acquisition", "governance_file": str(destination / "governance.json"),
               "authentication_file": str(destination / "authentication.json"),
               "privacy_inspector_file": str(destination / "privacy-inspector.json"), "max_active": 2,
               "exports": [{"agent": "collector", "name": "Deployment point collector",
                            "description": "Synthetic governed acquisition", "public": False}]}
    authentication = {"version": "mas-host-authentication/1", "principals": [
        {"tenant": "synthetic-tenant", "principal": "synthetic-user", "purpose": "synthetic-acquisition",
         "grant_ref": "synthetic-grant", "agents": ["collector"],
         "token_sha256": hashlib.sha256(b"synthetic-acquisition-local-token-0123456789").hexdigest()}]}
    for name, value in (("input-schema.json", input_schema), ("output-schema.json", output_schema),
                        ("peer-input-schema.json", peer_input), ("peer-output-schema.json", peer_output),
                        ("governance.json", governance), ("authority.json", authority),
                        ("privacy-inspector.json", inspector), ("gateway.json", gateway),
                        ("authentication.json", authentication), ("input.json", {"mode": "acquire"}),
                        ("topology.json", {"version": "mas-topology/1", "edges": [
                            ["client", "collector"], ["collector", "client"],
                            ["collector", "permitted"], ["permitted", "collector"],
                            ["collector", "blocked"], ["blocked", "collector"]]})):
        (destination / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    (destination / "commitment.key").write_bytes(os.urandom(32))
    (destination / "commitment.key").chmod(0o600)
    declarations = ['version = "mas-agents/3"']
    for actor in ("collector", "permitted", "blocked"):
        prefix = "" if actor == "collector" else "peer-"
        declarations.append(f'''[[agents]]
name = "{actor}"
operation = "fixture-{actor}/1"
input_schema = "{prefix}input-schema.json"
output_schema = "{prefix}output-schema.json"
[agents.implementation]
kind = "native"
language = "python"
source = "application.py"
handler = "{actor}"''')
    declarations.append('''[limits]
wall_seconds = 300
max_calls = 6
max_messages = 24
max_bytes = 262144
mailbox_capacity = 8''')
    (destination / "agents.toml").write_text("\n".join(declarations) + "\n", encoding="utf-8")
    return destination


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--destination", required=True)
    parser.add_argument("--fixture-python", default="/usr/bin/python3")
    args = parser.parse_args()
    print(prepare(args.destination, args.fixture_python))
