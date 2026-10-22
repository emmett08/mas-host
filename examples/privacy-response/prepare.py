"""Prepare one explicitly synthetic trusted-local governed response application."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]


def object_schema(properties, required=None):
    return {"type": "object", "properties": properties, "required": list(required or properties),
            "additionalProperties": False}


def enum(*values):
    return {"type": "string", "enum": list(values)}


def prepare(destination, fixture_python="/usr/bin/python3"):
    destination = Path(destination).resolve()
    destination.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(ROOT / "application.py", destination / "application.py")
    shutil.copyfile(ROOT / "fixture_inspector.py", destination / "fixture_inspector.py")
    shutil.copytree(REPO / "adapters/notifications/privacy_response", destination / "privacy_response",
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    modes = enum("accepted", "throttled", "unknown", "revoke-before-notify")
    input_schema = object_schema({"mode": modes})
    collector_output = object_schema({"approved": {"type": "boolean"}})
    output_schema = object_schema({
        "submission": enum("accepted", "refused", "not-sent", "throttled"),
        "recipient_delivery": enum("unknown"),
        "retry_after_seconds": {"type": "integer", "minimum": 0, "maximum": 3600},
        "response_metadata": enum("available", "withheld"),
    })
    event_schema = object_schema({
        "version": enum("mas-privacy-case-event/1"),
        "event_version": {"type": "integer", "minimum": 1, "maximum": 1_000_000},
        "reason": enum("handling-withheld"), "disposition": enum("withheld"),
        "exposure": enum("blocked"),
        "categories": {"type": "array", "items": enum("name", "address", "email", "phone", "account", "identifier", "credential", "location"),
                       "minItems": 0, "maxItems": 8},
        "finding_count": {"type": "integer", "minimum": 0, "maximum": 128},
        "route": enum("data-owner"),
    })
    case_output = object_schema({"case_retained": {"type": "boolean"}, "notification_pending": {"type": "boolean"}})
    case_evidence = object_schema({"execution": enum("completed"), "execution_scope": enum("host-case-intent")})
    notification_evidence = object_schema({"execution": enum("completed", "refused", "not-sent"),
                                           "execution_scope": enum("notification-endpoint-submission")})
    target_binding = enum(hashlib.sha256(b'mas-privacy-fixture-incident-endpoint/1').hexdigest())
    effects = [
        {"format": "mas-privacy-case-intent/1", "destination": "host-case-journal",
         "descriptor_schema": object_schema({"format": enum("mas-privacy-case-intent/1"), "event": event_schema,
                                              "target_binding": target_binding}),
         "output_schema": case_output, "evidence_schema": case_evidence},
        {"format": "mas-privacy-notification-submit/1", "destination": "fixture-incident-endpoint",
         "descriptor_schema": object_schema({"format": enum("mas-privacy-notification-submit/1"), "event": event_schema,
                                              "target_binding": target_binding}),
         "output_schema": output_schema, "evidence_schema": notification_evidence},
    ]
    governance = {"version": "mas-governed-local/1", "authority_file": str(destination / "authority.json"),
                  "commitment_key_file": str(destination / "commitment.key"),
                  "commitment_key_id": "synthetic-response-key-1", "location": "local",
                  "agents": [
                      {"actor": "responder", "destination": "local-worker", "input_schema": input_schema,
                       "output_schema": output_schema, "effects": effects},
                      {"actor": "collector", "destination": "local-worker", "input_schema": input_schema,
                       "output_schema": collector_output, "effects": [
                           {"format": "synthetic-private-collection/1", "destination": "fixture-source",
                            "descriptor_schema": object_schema({"format": enum("synthetic-private-collection/1")}),
                            "output_schema": collector_output,
                            "evidence_schema": object_schema({"execution": enum("completed")})}]}]}
    # This explicitly synthetic authority permits the restricted safe metadata
    # and immutable inline retention used by the example. No managed profile,
    # free-text release, endpoint credential or deployment authority is granted.
    rules = []
    for actor, actor_effects in (("responder", effects), ("collector", governance["agents"][1]["effects"])):
        bindings = [("invoke", "worker:" + actor, "local-worker"),
                    ("deliver", "synthetic-user", "client"),
                    ("inspect", "privacy-inspector", "local-detector")]
        bindings += [("effect", "operation:" + effect["format"], effect["destination"])
                     for effect in actor_effects]
        if actor == "collector":
            bindings.append(("invoke", "worker:responder", "local-worker"))
        for operation, recipient, target in bindings:
            rules.append({"operation": operation, "principal": "synthetic-user", "actor": actor,
                          "producer": "host", "recipient": recipient, "purpose": "synthetic-response",
                          "destination": target, "location": "local", "classifications": ["restricted"],
                          "obligations": ["inline-retention-approved"], "max_bytes": 65536})
    policy = {"version": "mas-data-use-policy/1", "id": "synthetic-response-policy",
              "tenant": "synthetic-tenant", "policy_version": "1", "valid_from_ms": 0,
              "expires_at_ms": 4102444800000, "rules": rules}
    authority = {"version": "mas-governed-authority/1", "grants": [
        {"version": "mas-current-authority/1", "status": "active", "tenant": "synthetic-tenant",
         "principal": "synthetic-user", "grant_ref": "synthetic-grant", "grant_epoch": 1,
         "policy_version": "1", "valid_from_ms": 0, "expires_at_ms": 4102444800000}], "policies": [policy]}
    worker = (destination / "fixture_inspector.py").read_bytes()
    # The fixture needs only Python's standard library. Select its interpreter
    # explicitly: the invoking analysis runtime may be a much larger embedded
    # executable, whose required per-launch identity check consumes its own
    # boundary budget. Real detector environments retain their independent pins.
    python = Path(fixture_python)
    if not python.is_absolute():
        raise ValueError("synthetic-fixture-python-requires-absolute-path")
    python = python.resolve(strict=True)
    checked = subprocess.run([str(python), "-I", "-c", "import sys; assert sys.version_info >= (3, 11)"],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
    if checked.returncode:
        raise ValueError("synthetic-fixture-python-requires-3.11-or-later")
    manifest = {"version": "mas-privacy-bundle/1", "code_revision": "a" * 40,
                "model_revision": "b" * 40, "backend": "opf-cpu-bfloat16/1", "adapter": "mas-privacy-opf/1",
                "worker_sha256": hashlib.sha256(worker).hexdigest(), "adapter_files": {}, "files": {}, "runtime": {},
                "settings": {"max_input_bytes": 16384, "max_response_bytes": 65536, "max_findings": 128}}
    manifest_raw = json.dumps(manifest, separators=(",", ":"), sort_keys=True).encode()
    (destination / "inspection-manifest.json").write_bytes(manifest_raw)
    inspector = {"version": "mas-governed-privacy-inspector/1", "python": str(python),
                 "python_sha256": hashlib.sha256(python.read_bytes()).hexdigest(),
                 "worker": str(destination / "fixture_inspector.py"),
                 "manifest": str(destination / "inspection-manifest.json"), "assets": str(destination),
                 "detector": {"bundle_sha256": hashlib.sha256(manifest_raw).hexdigest(),
                              "code_revision": "a" * 40, "model_revision": "b" * 40,
                              "backend": "opf-cpu-bfloat16/1", "adapter": "mas-privacy-opf/1"},
                 "destination": "local-detector", "timeout_ms": 30000, "memory_mib": 256}
    gateway = {"version": "mas-host-a2a/2", "listen": "127.0.0.1:8787", "public_url": "http://127.0.0.1:8787/a2a",
               "name": "Synthetic privacy response", "governance_file": str(destination / "governance.json"),
               "authentication_file": str(destination / "authentication.json"),
               "privacy_inspector_file": str(destination / "privacy-inspector.json"), "max_active": 2,
               "exports": [{"agent": "responder", "name": "Privacy response",
                            "description": "Synthetic handling and response", "public": False}]}
    authentication = {"version": "mas-host-authentication/1", "principals": [
        {"tenant": "synthetic-tenant", "principal": "synthetic-user", "purpose": "synthetic-response",
         "grant_ref": "synthetic-grant", "agents": ["responder"],
         "token_sha256": hashlib.sha256(b"synthetic-response-local-token-0123456789").hexdigest()}]}
    for name, value in (("input-schema.json", input_schema), ("output-schema.json", output_schema),
                        ("collector-output-schema.json", collector_output), ("governance.json", governance),
                        ("authority.json", authority), ("input.json", {"mode": "accepted"}),
                        ("privacy-inspector.json", inspector), ("gateway.json", gateway),
                        ("authentication.json", authentication),
                        ("topology.json", {"version": "mas-topology/1", "edges": [
                            ["client", "responder"], ["responder", "client"],
                            ["responder", "collector"], ["collector", "responder"]]})):
        (destination / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    (destination / "commitment.key").write_bytes(os.urandom(32))
    (destination / "commitment.key").chmod(0o600)
    (destination / "agents.toml").write_text('''version = "mas-agents/3"
[[agents]]
name = "responder"
operation = "privacy-response/1"
input_schema = "input-schema.json"
output_schema = "output-schema.json"
[agents.implementation]
kind = "native"
language = "python"
source = "application.py"
handler = "respond"
[[agents]]
name = "collector"
operation = "synthetic-private-collection/1"
input_schema = "input-schema.json"
output_schema = "collector-output-schema.json"
[agents.implementation]
kind = "native"
language = "python"
source = "application.py"
handler = "collect"
[limits]
wall_seconds = 300
max_calls = 4
max_messages = 16
max_bytes = 262144
mailbox_capacity = 8
''', encoding="utf-8")
    return destination


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--destination", required=True)
    parser.add_argument("--fixture-python", default="/usr/bin/python3")
    arguments = parser.parse_args()
    print(prepare(arguments.destination, arguments.fixture_python))
